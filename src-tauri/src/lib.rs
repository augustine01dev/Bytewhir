mod db;
mod fetcher;

use fetcher::{fetch_arxiv, fetch_github, fetch_hn, fetch_rss, DigestItem};
use rusqlite::Connection;
use std::collections::HashSet;
use std::sync::Mutex;
use tauri::{Manager, State};

struct AppState {
    db_conn: Mutex<Connection>,
}

#[tauri::command]
async fn fetch_digest(state: State<'_, AppState>) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let enabled_sources: HashSet<String> = {
        let conn = state.db_conn.lock().unwrap();
        db::get_enabled_source_ids(&conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect()
    };

    let github_enabled = enabled_sources.contains("github");
    let hacker_news_enabled = enabled_sources.contains("hackernews");
    let arxiv_enabled = enabled_sources.contains("arxiv");
    let techcrunch_enabled = enabled_sources.contains("techcrunch");

    let (hn, gh, arxiv, rss) = tokio::join!(
        async {
            if hacker_news_enabled {
                fetch_hn(&client).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if github_enabled {
                fetch_github(&client).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if arxiv_enabled {
                fetch_arxiv(&client).await
            } else {
                Ok(Vec::new())
            }
        },
        async {
            if techcrunch_enabled {
                fetch_rss(&client).await
            } else {
                Ok(Vec::new())
            }
        }
    );

    let mut items = Vec::new();
    items.append(&mut hn.unwrap_or_default());
    items.append(&mut gh.unwrap_or_default());
    items.append(&mut arxiv.unwrap_or_default());
    items.append(&mut rss.unwrap_or_default());

    // Upsert into DB
    let conn = state.db_conn.lock().unwrap();
    db::upsert_items(&conn, &items).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn get_digest(
    state: State<'_, AppState>,
    category: Option<String>,
    before_timestamp: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<DigestItem>, String> {
    let conn = state.db_conn.lock().unwrap();
    let limit_val = limit.unwrap_or(20);
    db::get_items(&conn, category, before_timestamp, limit_val).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_saved_digest(
    state: State<'_, AppState>,
    before_timestamp: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<DigestItem>, String> {
    let conn = state.db_conn.lock().unwrap();
    db::get_starred_items(&conn, before_timestamp, limit.unwrap_or(20)).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_latest_fetched_at(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let conn = state.db_conn.lock().unwrap();
    db::get_latest_fetched_at(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_starred(state: State<'_, AppState>, id: String) -> Result<i32, String> {
    let conn = state.db_conn.lock().unwrap();
    db::toggle_starred(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_sources(state: State<'_, AppState>) -> Result<Vec<db::Source>, String> {
    let conn = state.db_conn.lock().unwrap();
    db::get_sources(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn toggle_source(state: State<'_, AppState>, id: String) -> Result<db::Source, String> {
    let conn = state.db_conn.lock().unwrap();
    db::toggle_source(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command]
fn mark_seen(state: State<'_, AppState>, ids: Vec<String>) -> Result<(), String> {
    let conn = state.db_conn.lock().unwrap();
    db::mark_seen(&conn, &ids).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_old_items(state: State<'_, AppState>, days: u32) -> Result<(), String> {
    let conn = state.db_conn.lock().unwrap();
    db::clear_old_items(&conn, days).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");
            let conn = db::init_db(&app_data_dir).expect("Failed to initialize database");
            app.manage(AppState {
                db_conn: Mutex::new(conn),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fetch_digest,
            get_digest,
            get_saved_digest,
            get_latest_fetched_at,
            toggle_starred,
            get_sources,
            toggle_source,
            mark_seen,
            clear_old_items
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
