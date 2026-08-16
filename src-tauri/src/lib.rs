use tauri::{Emitter, Manager, WindowEvent, WebviewUrl, WebviewWindowBuilder, tray::{TrayIconBuilder, MouseButton, MouseButtonState}};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use std::sync::atomic::{AtomicU16, AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static GATEWAY_PORT: AtomicU16 = AtomicU16::new(0);
static STARTING: AtomicBool = AtomicBool::new(false);
static SUPPRESS_RESTART: AtomicBool = AtomicBool::new(false);
static CRASH_COUNT: AtomicU8 = AtomicU8::new(0);

struct GatewayChild(Mutex<Option<tauri_plugin_shell::process::CommandChild>>);
impl Drop for GatewayChild {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
}

fn find_gateway_port() -> u16 {
    if std::net::TcpListener::bind("127.0.0.1:3000").is_ok() {
        return 3000;
    }
    if let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    3000
}

fn port_is_listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        Duration::from_millis(300),
    ).is_ok()
}

fn wait_for_port(port: u16, timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < timeout_secs {
        if port_is_listening(port) { return true; }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn wait_for_port_release(port: u16, timeout_secs: u64) {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < timeout_secs {
        if !port_is_listening(port) { return; }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn beijing_timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let bj = secs + 8 * 3600;
    let days = (bj / 86400) as i64;
    let tod = bj % 86400;
    format!("{}D {:02}:{:02}:{:02}", days, tod / 3600, (tod % 3600) / 60, tod % 60)
}

fn log_to_file(app_data: &std::path::Path, msg: &str) {
    let log_dir = app_data.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join("app.log");
    let line = format!("[{}] {}\n", beijing_timestamp(), msg);
    let _ = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(log_file)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

fn kill_existing_gateway(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<GatewayChild>() {
        if let Ok(mut guard) = state.0.lock() {
            if let Some(child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
}

fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        match std::process::Command::new("explorer").arg(url).spawn() {
            Ok(_) => return Ok(()),
            Err(e1) => {
                match std::process::Command::new("cmd")
                    .args(["/C", "start", "", url]).spawn() {
                    Ok(_) => return Ok(()),
                    Err(e2) => return Err(format!("打开浏览器失败: explorer={}, cmd={}", e1, e2)),
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()
            .map_err(|e| format!("打开浏览器失败: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()
            .map_err(|e| format!("打开浏览器失败: {}", e))?;
    }
    #[allow(unreachable_code)]
    Ok(())
}

fn start_gateway_internal(app: &tauri::AppHandle) -> Result<u16, String> {
    let existing_port = GATEWAY_PORT.load(Ordering::SeqCst);
    if existing_port != 0 && port_is_listening(existing_port) {
        return Ok(existing_port);
    }
    if STARTING.load(Ordering::SeqCst) {
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            let p = GATEWAY_PORT.load(Ordering::SeqCst);
            if p != 0 { return Ok(p); }
        }
        return Err("网关正在启动中".to_string());
    }
    STARTING.store(true, Ordering::SeqCst);
    SUPPRESS_RESTART.store(false, Ordering::SeqCst);
    GATEWAY_PORT.store(0, Ordering::SeqCst);
    kill_existing_gateway(app);
    let port = find_gateway_port();
    let app_data = app.path().app_data_dir()
        .map_err(|e| { STARTING.store(false, Ordering::SeqCst); format!("无法获取数据目录: {}", e) })?;
    let data_dir = app_data.join("gateway");
    let _ = std::fs::create_dir_all(&data_dir);
    let logs_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&logs_dir);
    let port_str = port.to_string();
    let log_dir_str = logs_dir.to_string_lossy().to_string();
    log_to_file(&app_data, &format!("启动网关，端口: {}", port));
    let sidecar = app.shell().sidecar("one-api")
        .map_err(|e| { let m=format!("网关程序丢失，请重新安装AI作战室（sidecar缺失: {}）",e); log_to_file(&app_data,&m); STARTING.store(false,Ordering::SeqCst); m })?
        .args(["--port", &port_str, "--log-dir", &log_dir_str])
        .current_dir(&data_dir);
    let (mut rx, child) = sidecar.spawn()
        .map_err(|e| { let m=format!("网关启动失败，请尝试重启应用或重新安装（错误: {}）",e); log_to_file(&app_data,&m); STARTING.store(false,Ordering::SeqCst); m })?;
    if let Some(state) = app.try_state::<GatewayChild>() {
        if let Ok(mut guard) = state.0.lock() { *guard = Some(child); }
    }
    let app_h = app.clone();
    let ad = app_data.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    let trimmed = text.trim().to_string();
                    log_to_file(&ad, &format!("[stdout] {}", trimmed));
                    let _ = app_h.emit("gateway-stdout", text);
                    if trimmed.contains("server started") {
                        CRASH_COUNT.store(0, Ordering::SeqCst);
                    }
                }
                CommandEvent::Stderr(line) => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    let trimmed = text.trim().to_string();
                    log_to_file(&ad, &format!("[stderr] {}", trimmed));
                    let _ = app_h.emit("gateway-stderr", text);
                }
                CommandEvent::Error(err) => {
                    log_to_file(&ad, &format!("[error] {}", err));
                    let _ = app_h.emit("gateway-error", err.to_string());
                }
                CommandEvent::Terminated(payload) => {
                    let code = payload.code.unwrap_or(-1);
                    log_to_file(&ad, &format!("[terminated] exit code: {}", code));
                    let _ = app_h.emit("gateway-exit", code);
                    GATEWAY_PORT.store(0, Ordering::SeqCst);
                    STARTING.store(false, Ordering::SeqCst);
                    if !SUPPRESS_RESTART.load(Ordering::SeqCst) && code != 0 {
                        let count = CRASH_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
                        if count <= 3 {
                            log_to_file(&ad, &format!("网关异常退出，3秒后自动重启（第{}次）", count));
                            let a = app_h.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(3));
                                let _ = start_gateway_internal(&a);
                            });
                        } else {
                            log_to_file(&ad, "连续崩溃3次，停止自动重启");
                            let _ = app_h.emit("gateway-error", "网关反复崩溃（已自动重启3次），可能是数据损坏或杀毒软件拦截，请查看日志或重新安装。");
                        }
                    }
                }
                _ => {}
            }
        }
    });
    let ah2 = app.clone();
    let ad2 = app_data.clone();
    std::thread::spawn(move || {
        if wait_for_port(port, 20) {
            GATEWAY_PORT.store(port, Ordering::SeqCst);
            STARTING.store(false, Ordering::SeqCst);
            log_to_file(&ad2, &format!("网关就绪，端口: {}", port));
            let _ = ah2.emit("gateway-ready", format!("http://127.0.0.1:{}", port));
        } else {
            STARTING.store(false, Ordering::SeqCst);
            log_to_file(&ad2, "网关启动超时");
            let _ = ah2.emit("gateway-error", "网关启动超时，可能是端口被占用或杀毒软件拦截，请尝试重启应用。");
        }
    });
    Ok(port)
}

#[tauri::command]
async fn start_gateway(app: tauri::AppHandle) -> Result<String, String> {
    let port = start_gateway_internal(&app)?;
    Ok(format!("端口: {}", port))
}

#[tauri::command]
fn get_gateway_port() -> u16 {
    GATEWAY_PORT.load(Ordering::SeqCst)
}

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

fn dashboard_url() -> String {
    let port = GATEWAY_PORT.load(Ordering::SeqCst);
    if port > 0 {
        format!("http://127.0.0.1:{}", port)
    } else {
        "http://127.0.0.1:3000".to_string()
    }
}

/// v0.4.2修复：async命令中直接创建窗口，不使用spawn_blocking。
/// Tauri 2 Windows已知bug：同步命令中WebviewWindowBuilder.build()会死锁。
/// 官方推荐做法：async fn + 直接调用build()，async命令在线程池上运行，
/// 主线程消息泵保持畅通，WebView2可正常初始化。
#[tauri::command]
async fn open_dashboard(app: tauri::AppHandle) -> Result<(), String> {
    let app_data = app.path().app_data_dir().unwrap_or_default();
    log_to_file(&app_data, "open_dashboard命令被调用");
    let _ = start_gateway_internal(&app);
    let url = dashboard_url();
    log_to_file(&app_data, &format!("打开管理面板(内嵌): {}", url));

    if let Some(existing) = app.get_webview_window("dashboard") {
        let _ = existing.show();
        let _ = existing.set_focus();
        log_to_file(&app_data, "管理面板窗口已存在，已聚焦");
        return Ok(());
    }

    let url_parsed: tauri::Url = url.parse().map_err(|e| format!("URL解析失败: {}", e))?;
    match WebviewWindowBuilder::new(&app, "dashboard", WebviewUrl::External(url_parsed))
        .title("AI作战室 - 管理面板")
        .inner_size(1200.0, 800.0)
        .min_inner_size(900.0, 600.0)
        .resizable(true)
        .center()
        .visible(true)
        .build()
    {
        Ok(_) => {
            log_to_file(&app_data, "管理面板内嵌窗口已创建");
            Ok(())
        }
        Err(e) => {
            log_to_file(&app_data, &format!("内嵌窗口创建失败，回退浏览器: {}", e));
            let _ = start_gateway_internal(&app);
            open_in_browser(&url).map_err(|e2| format!("内嵌失败({})，浏览器也失败: {}", e, e2))
        }
    }
}

#[tauri::command]
fn restart_gateway(app: tauri::AppHandle) -> Result<(), String> {
    SUPPRESS_RESTART.store(true, Ordering::SeqCst);
    kill_existing_gateway(&app);
    GATEWAY_PORT.store(0, Ordering::SeqCst);
    STARTING.store(false, Ordering::SeqCst);
    let app2 = app.clone();
    std::thread::spawn(move || {
        wait_for_port_release(3000, 3);
        std::thread::sleep(Duration::from_millis(500));
        let _ = start_gateway_internal(&app2);
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(GatewayChild(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            start_gateway,
            get_gateway_port,
            get_app_version,
            open_dashboard,
            restart_gateway
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .setup(|app| {
            let ah = app.handle().clone();
            if let Err(e) = start_gateway_internal(&ah) {
                eprintln!("启动网关失败: {}", e);
            }
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("AI作战室")
                .menu(&tauri::menu::Menu::with_items(app.handle(), &[
                    &tauri::menu::MenuItem::with_id(app.handle(), "show", "显示窗口", true, None::<&str>)?,
                    &tauri::menu::MenuItem::with_id(app.handle(), "dashboard", "打开管理面板", true, None::<&str>)?,
                    &tauri::menu::MenuItem::with_id(app.handle(), "restart", "重启网关", true, None::<&str>)?,
                    &tauri::menu::PredefinedMenuItem::separator(app.handle())?,
                    &tauri::menu::MenuItem::with_id(app.handle(), "viewlog", "查看日志", true, None::<&str>)?,
                    &tauri::menu::MenuItem::with_id(app.handle(), "quit", "退出", true, None::<&str>)?,
                ])?)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); }
                    }
                    "dashboard" => {
                        // v0.4.2: 托盘菜单用tauri::async_runtime::spawn，不用std::thread::spawn
                        let app_clone = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let url = dashboard_url();
                            let app_data = app_clone.path().app_data_dir().unwrap_or_default();
                            log_to_file(&app_data, &format!("托盘打开管理面板(内嵌): {}", url));
                            if let Some(existing) = app_clone.get_webview_window("dashboard") {
                                let _ = existing.show();
                                let _ = existing.set_focus();
                                return;
                            }
                            if let Ok(url_parsed) = url.parse::<tauri::Url>() {
                                match WebviewWindowBuilder::new(&app_clone, "dashboard", WebviewUrl::External(url_parsed))
                                    .title("AI作战室 - 管理面板")
                                    .inner_size(1200.0, 800.0)
                                    .min_inner_size(900.0, 600.0)
                                    .resizable(true)
                                    .center()
                                    .visible(true)
                                    .build()
                                {
                                    Ok(_) => log_to_file(&app_data, "托盘: 管理面板内嵌窗口已创建"),
                                    Err(e) => {
                                        log_to_file(&app_data, &format!("托盘: 内嵌窗口创建失败: {}", e));
                                        let _ = open_in_browser(&url);
                                    }
                                }
                            }
                        });
                    }
                    "restart" => {
                        SUPPRESS_RESTART.store(true, Ordering::SeqCst);
                        kill_existing_gateway(app);
                        GATEWAY_PORT.store(0, Ordering::SeqCst);
                        STARTING.store(false, Ordering::SeqCst);
                        let a = app.clone();
                        std::thread::spawn(move || {
                            wait_for_port_release(3000, 3);
                            std::thread::sleep(Duration::from_millis(500));
                            let _ = start_gateway_internal(&a);
                        });
                    }
                    "viewlog" => {
                        let app_data = app.path().app_data_dir().unwrap_or_default();
                        let log_dir = app_data.join("logs");
                        let _ = std::fs::create_dir_all(&log_dir);
                        let log_path = log_dir.join("app.log");
                        log_to_file(&app_data, "打开日志文件");
                        #[cfg(target_os = "windows")]
                        {
                            let _ = std::process::Command::new("explorer")
                                .arg(&log_path)
                                .spawn();
                        }
                        #[cfg(target_os = "macos")]
                        {
                            let _ = std::process::Command::new("open")
                                .arg(&log_path)
                                .spawn();
                        }
                        #[cfg(target_os = "linux")]
                        {
                            let _ = std::process::Command::new("xdg-open")
                                .arg(&log_path)
                                .spawn();
                        }
                    }
                    "quit" => {
                        SUPPRESS_RESTART.store(true, Ordering::SeqCst);
                        kill_existing_gateway(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) { let _ = w.hide(); }
                            else { let _ = w.show(); let _ = w.set_focus(); }
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
