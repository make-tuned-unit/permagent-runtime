use tauri::Manager;

mod oauth;
mod pty;

fn check_daemon(port: u16) -> bool {
    std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

#[tauri::command]
fn daemon_status() -> bool {
    check_daemon(3000)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(pty::new_sessions())
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            pty::spawn_pty_session,
            pty::write_to_pty,
            pty::resize_pty,
            pty::kill_pty,
        ])
        .setup(|_app| {
            if !check_daemon(3000) {
                eprintln!("Warning: permagentd not detected on port 3000");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
