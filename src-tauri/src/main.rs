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

    let tauri_result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!());

    // Dev convenience: when GALVOLITE_KEEP_API=1 is set, keep the API
    // server thread alive even after the Tauri window closes. Lets a
    // browser-based reload test (chrome on localhost:5173) survive
    // accidental window closes during debugging.
    if std::env::var("GALVOLITE_KEEP_API").is_ok() {
        if let Err(e) = &tauri_result {
            eprintln!("tauri exited ({e}); GALVOLITE_KEEP_API set, parking forever");
        } else {
            eprintln!("tauri window closed; GALVOLITE_KEEP_API set, parking forever");
        }
        std::thread::park();
    }

    tauri_result.expect("error while running tauri application");
}
