use tauri::{Emitter, Manager, tray::{TrayIconBuilder, MouseButton, MouseButtonState}};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static GATEWAY_PORT: AtomicU16 = AtomicU16::new(0);

struct GatewayChild(Mutex<Option<tauri_plugin_shell::process::CommandChild>>);

/// 找一个可用端口
fn find_available_port() -> u16 {
    if let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    3000
}

/// 等待端口就绪
fn wait_for_port(port: u16, timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < timeout_secs {
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(200),
        ).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// 写日志到文件
fn log_to_file(app_data: &std::path::Path, msg: &str) {
    let log_dir = app_data.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join("app.log");
    let timestamp = {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let days = (secs / 86400) as i64;
        let time_of_day = secs % 86400;
        let h = time_of_day / 3600;
        let m = (time_of_day % 3600) / 60;
        let s = time_of_day % 60;
        format!("{}D {:02}:{:02}:{:02}", days, h, m, s)
    };
    let line = format!("[{}] {}\n", timestamp, msg);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// 启动One API网关
fn start_gateway_internal(app: &tauri::AppHandle) -> Result<u16, String> {
    let existing_port = GATEWAY_PORT.load(Ordering::SeqCst);
    if existing_port != 0 {
        return Ok(existing_port);
    }

    let port = find_available_port();
    let app_data = app.path().app_data_dir()
        .map_err(|e| format!("无法获取数据目录: {}", e))?;
    let data_dir = app_data.join("gateway");
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("无法创建数据目录: {}", e))?;

    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)
        .map_err(|e| format!("无法创建日志目录: {}", e))?;

    let port_str = port.to_string();
    let log_dir_str = logs_dir.to_string_lossy().to_string();

    log_to_file(&app_data, &format!("启动网关，端口: {}", port));

    let shell = app.shell();
    let sidecar = shell.sidecar("one-api")
        .map_err(|e| {
            let msg = format!("无法找到sidecar: {}", e);
            log_to_file(&app_data, &msg);
            msg
        })?
        .args(["--port", &port_str, "--log-dir", &log_dir_str])
        .current_dir(&data_dir);

    let (mut rx, child) = sidecar.spawn()
        .map_err(|e| {
            let msg = format!("启动网关失败: {}", e);
            log_to_file(&app_data, &msg);
            msg
        })?;

    // 保存child句柄，防止被drop导致进程被杀
    if let Some(state) = app.try_state::<GatewayChild>() {
        *state.0.lock().unwrap() = Some(child);
    }

    // 监听sidecar输出
    let app_handle = app.clone();
    let app_data_clone = app_data.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    log_to_file(&app_data_clone, &format!("[stdout] {}", text.trim()));
                    let _ = app_handle.emit("gateway-stdout", text.clone());
                    if text.contains("server started") || text.contains("listening") {
                        let _ = app_handle.emit("gateway-ready", format!("http://127.0.0.1:{}", port));
                    }
                }
                CommandEvent::Stderr(line) => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    log_to_file(&app_data_clone, &format!("[stderr] {}", text.trim()));
                    let _ = app_handle.emit("gateway-stderr", text.clone());
                    if text.contains("server started") || text.contains("listening") || text.contains("One API") {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        let _ = app_handle.emit("gateway-ready", format!("http://127.0.0.1:{}", port));
                    }
                }
                CommandEvent::Error(err) => {
                    log_to_file(&app_data_clone, &format!("[error] {}", err));
                    let _ = app_handle.emit("gateway-error", err.to_string());
                }
                CommandEvent::Terminated(payload) => {
                    let code = payload.code.unwrap_or(-1);
                    log_to_file(&app_data_clone, &format!("[terminated] exit code: {}", code));
                    let _ = app_handle.emit("gateway-exit", code);
                    GATEWAY_PORT.store(0, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    });

    // 后台线程等待端口就绪
    let app_handle2 = app.clone();
    let app_data2 = app_data.clone();
    std::thread::spawn(move || {
        if wait_for_port(port, 20) {
            GATEWAY_PORT.store(port, Ordering::SeqCst);
            log_to_file(&app_data2, &format!("网关就绪，端口: {}", port));
            let _ = app_handle2.emit("gateway-ready", format!("http://127.0.0.1:{}", port));
        } else {
            log_to_file(&app_data2, "网关启动超时，端口未就绪");
            let _ = app_handle2.emit("gateway-error", "网关启动超时，请检查日志");
        }
    });

    Ok(port)
}

#[tauri::command]
async fn start_gateway(app: tauri::AppHandle) -> Result<String, String> {
    let port = start_gateway_internal(&app)?;
    Ok(format!("网关启动中，端口: {}", port))
}

#[tauri::command]
fn get_gateway_port() -> u16 {
    GATEWAY_PORT.load(Ordering::SeqCst)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(GatewayChild(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            start_gateway,
            get_gateway_port
        ])
        .setup(|app| {
            // 在setup阶段直接启动网关，不依赖前端
            let app_handle = app.handle().clone();
            if let Err(e) = start_gateway_internal(&app_handle) {
                eprintln!("启动网关失败: {}", e);
            }

            // 托盘图标
            let app_handle_tray = app.handle().clone();
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("AI作战室")
                .menu(&tauri::menu::Menu::with_items(
                    app.handle(),
                    &[
                        &tauri::menu::MenuItem::with_id(app.handle(), "show", "显示窗口", true, None::<&str>)?,
                        &tauri::menu::MenuItem::with_id(app.handle(), "restart", "重启网关", true, None::<&str>)?,
                        &tauri::menu::MenuItem::with_id(app.handle(), "quit", "退出", true, None::<&str>)?,
                    ],
                )?)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "restart" => {
                        GATEWAY_PORT.store(0, Ordering::SeqCst);
                        let app = app.clone();
                        std::thread::spawn(move || {
                            let _ = start_gateway_internal(&app);
                        });
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
