use tauri::Manager;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use std::sync::atomic::{AtomicU16, Ordering};
use std::net::TcpListener;

static GATEWAY_PORT: AtomicU16 = AtomicU16::new(0);

#[cfg(target_os = "windows")]
const SIDECAR_NAME: &str = "one-api-x86_64-pc-windows-msvc.exe";
#[cfg(target_os = "linux")]
const SIDECAR_NAME: &str = "one-api-x86_64-unknown-linux-gnu";
#[cfg(target_os = "macos")]
const SIDECAR_NAME: &str = "one-api-aarch64-apple-darwin";

/// 找一个可用端口
fn find_available_port() -> u16 {
    // 尝试绑定到端口0让OS分配
    if let Ok(listener) = TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    // 兜底：3000
    3000
}

/// 启动One API网关sidecar
#[tauri::command]
async fn start_gateway(app: tauri::AppHandle) -> Result<String, String> {
    // 检查是否已启动
    let existing_port = GATEWAY_PORT.load(Ordering::SeqCst);
    if existing_port != 0 {
        return Ok(format!("网关已在运行，端口: {}", existing_port));
    }

    // 找可用端口
    let port = find_available_port();
    GATEWAY_PORT.store(port, Ordering::SeqCst);

    // 数据目录：app_data_dir/one-api-data
    // One API的SQLite文件存在工作目录，所以需要把sidecar的CWD设到这里
    let app_data = app.path().app_data_dir()
        .map_err(|e| format!("无法获取数据目录: {}", e))?;
    let data_dir = app_data.join("gateway");
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("无法创建数据目录: {}", e))?;

    let logs_dir = app_data.join("logs");
    std::fs::create_dir_all(&logs_dir)
        .map_err(|e| format!("无法创建日志目录: {}", e))?;

    let port_str = port.to_string();
    let log_dir_str = logs_dir.to_string_lossy().to_string();

    // 启动sidecar
    // One API启动参数：--port 指定端口，--log-dir 指定日志目录
    // 数据(SQLite)存在进程工作目录(CWD)，通过current_dir设置
    let shell = app.shell();
    let sidecar = shell.sidecar("one-api")
        .map_err(|e| format!("无法找到sidecar: {}", e))?
        .args(["--port", &port_str, "--log-dir", &log_dir_str])
        .current_dir(&data_dir);

    let (mut rx, _child) = sidecar.spawn()
        .map_err(|e| format!("启动网关失败: {}", e))?;

    // 监听sidecar输出，检测启动完成
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut ready_emitted = false;
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    if !ready_emitted && (text.contains("started") || text.contains("listening")) {
                        let url = format!("http://127.0.0.1:{}", GATEWAY_PORT.load(Ordering::SeqCst));
                        let _ = app_handle.emit("gateway-ready", url.clone());
                        ready_emitted = true;
                    }
                    let _ = app_handle.emit("gateway-stdout", text);
                }
                CommandEvent::Stderr(line) => {
                    let text = String::from_utf8_lossy(&line).to_string();
                    if !ready_emitted && (text.contains("started") || text.contains("listening") || text.contains("One API")) {
                        // 给HTTP服务一点时间完全就绪
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        let url = format!("http://127.0.0.1:{}", GATEWAY_PORT.load(Ordering::SeqCst));
                        let _ = app_handle.emit("gateway-ready", url.clone());
                        ready_emitted = true;
                    }
                    let _ = app_handle.emit("gateway-stderr", text);
                }
                CommandEvent::Error(err) => {
                    let _ = app_handle.emit("gateway-error", err.to_string());
                }
                CommandEvent::Terminated(payload) => {
                    let code = payload.code.unwrap_or(-1);
                    let _ = app_handle.emit("gateway-exit", code);
                    GATEWAY_PORT.store(0, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    });

    // 兜底：5秒后如果还没emit ready，直接发一个
    let app_handle2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;
        if GATEWAY_PORT.load(Ordering::SeqCst) != 0 {
            let url = format!("http://127.0.0.1:{}", GATEWAY_PORT.load(Ordering::SeqCst));
            let _ = app_handle2.emit("gateway-ready-timeout", url);
        }
    });

    Ok(format!("网关启动中，端口: {}", port))
}

/// 停止网关
#[tauri::command]
async fn stop_gateway() -> Result<(), String> {
    // sidecar在Tauri应用退出时会自动终止
    // 这里重置端口状态
    GATEWAY_PORT.store(0, Ordering::SeqCst);
    Ok(())
}

/// 获取网关端口
#[tauri::command]
fn get_gateway_port() -> u16 {
    GATEWAY_PORT.load(Ordering::SeqCst)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            start_gateway,
            stop_gateway,
            get_gateway_port
        ])
        .setup(|_app| {
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
