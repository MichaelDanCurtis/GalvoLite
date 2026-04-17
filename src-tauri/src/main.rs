#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use galvolite::db::Db;
use galvolite::session_manager::SessionManager;
use std::sync::Arc;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("galvolite=info")),
        )
        .init();

    let data_dir = dirs::data_dir()
        .expect("could not determine OS data dir")
        .join("GalvoLite");
    std::fs::create_dir_all(&data_dir).expect("create data dir");

    let db = Db::open(data_dir.join("galvolite.sqlite")).expect("open database");
    let mgr = Arc::new(SessionManager::new(db, data_dir));

    let port: u16 = std::env::var("GALVOLITE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3001);

    let mgr_for_server = mgr.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            let app = galvolite::api::router(mgr_for_server);
            let addr = format!("127.0.0.1:{port}");
            tracing::info!("API server on http://{addr}");
            let listener = tokio::net::TcpListener::bind(&addr)
                .await
                .expect("bind port");
            axum::serve(listener, app).await.expect("axum serve");
        });
    });

    std::thread::sleep(std::time::Duration::from_millis(200));

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
