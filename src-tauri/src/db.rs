use crate::fetcher::DigestItem;
use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub enabled: i32,
}

const DEFAULT_SOURCES: [(&str, &str); 4] = [
    ("github", "GitHub Trending"),
    ("hackernews", "Hacker News"),
    ("arxiv", "ArXiv"),
    ("techcrunch", "TechCrunch"),
];

pub fn init_db(app_data_dir: &PathBuf) -> Result<Connection> {
    std::fs::create_dir_all(app_data_dir).ok();
    let db_path = app_data_dir.join("bytewhir.db");

    // Keep existing local data when the app is renamed. The app data directory
    // is private to this application, so an existing database file is the
    // previous local store and can be copied to the new branded filename.
    if !db_path.exists() {
        if let Ok(entries) = std::fs::read_dir(app_data_dir) {
            if let Some(previous_db) = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .find(|path| {
                    path.extension().and_then(|extension| extension.to_str()) == Some("db")
                })
            {
                let _ = std::fs::copy(previous_db, &db_path);
            }
        }
    }

    let conn = Connection::open(db_path)?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS digest_items (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            category TEXT,
            title TEXT NOT NULL,
            summary TEXT,
            thumbnail_url TEXT,
            url TEXT NOT NULL,
            published_at TEXT NOT NULL,
            fetched_at TEXT NOT NULL,
            seen INTEGER DEFAULT 0,
            starred INTEGER DEFAULT 0,
            thumbnail_resolved INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )?;

    // Existing local databases predate thumbnail caching. Keep them usable by
    // adding the flag once, so old items are resolved on their next fetch.
    let has_thumbnail_resolved: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('digest_items') WHERE name = 'thumbnail_resolved')",
        [],
        |row| row.get(0),
    )?;
    if !has_thumbnail_resolved {
        conn.execute(
            "ALTER TABLE digest_items ADD COLUMN thumbnail_resolved INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    conn.execute(
        "CREATE TABLE IF NOT EXISTS sources (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1
        )",
        [],
    )?;

    for (id, name) in DEFAULT_SOURCES {
        conn.execute(
            "INSERT OR IGNORE INTO sources (id, name, enabled) VALUES (?1, ?2, 1)",
            params![id, name],
        )?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_published_at ON digest_items(published_at DESC)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_category ON digest_items(category)",
        [],
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_starred_fetched_at ON digest_items(starred, fetched_at DESC)",
        [],
    )?;

    Ok(conn)
}

pub fn upsert_items(conn: &Connection, items: &[DigestItem]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO digest_items (
            id, source, category, title, summary, thumbnail_url, url, published_at, fetched_at, seen, starred, thumbnail_resolved
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12
        )
        ON CONFLICT(id) DO UPDATE SET
            summary = excluded.summary,
            fetched_at = excluded.fetched_at,
            thumbnail_url = CASE
                WHEN digest_items.thumbnail_resolved = 1 THEN digest_items.thumbnail_url
                ELSE excluded.thumbnail_url
            END,
            thumbnail_resolved = CASE
                WHEN digest_items.thumbnail_resolved = 1 THEN 1
                ELSE excluded.thumbnail_resolved
            END"
    )?;

    for item in items {
        stmt.execute(params![
            item.id,
            item.source,
            item.category,
            item.title,
            item.summary,
            item.thumbnail_url,
            item.url,
            item.published_at,
            item.fetched_at,
            item.seen,
            item.starred,
            item.thumbnail_resolved
        ])?;
    }
    Ok(())
}

pub fn get_items(
    conn: &Connection,
    category: Option<String>,
    before_timestamp: Option<String>,
    limit: u32,
) -> Result<Vec<DigestItem>> {
    let mut sql = "SELECT id, source, category, title, summary, thumbnail_url, url, published_at, fetched_at, seen, starred, thumbnail_resolved FROM digest_items WHERE 1=1".to_string();
    let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(cat) = category {
        if cat != "All" {
            sql.push_str(" AND category = ?");
            params_vec.push(rusqlite::types::Value::Text(cat));
        }
    }

    if let Some(ts) = before_timestamp {
        sql.push_str(" AND published_at < ?");
        params_vec.push(rusqlite::types::Value::Text(ts));
    }

    sql.push_str(" ORDER BY published_at DESC LIMIT ?");
    params_vec.push(rusqlite::types::Value::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let item_iter = stmt.query_map(rusqlite::params_from_iter(params_vec), digest_item_from_row)?;

    let mut items = Vec::new();
    for item in item_iter {
        items.push(item?);
    }
    Ok(items)
}

pub fn get_starred_items(
    conn: &Connection,
    before_timestamp: Option<String>,
    limit: u32,
) -> Result<Vec<DigestItem>> {
    let mut sql = "SELECT id, source, category, title, summary, thumbnail_url, url, published_at, fetched_at, seen, starred, thumbnail_resolved FROM digest_items WHERE starred = 1".to_string();
    let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(ts) = before_timestamp {
        sql.push_str(" AND fetched_at < ?");
        params_vec.push(rusqlite::types::Value::Text(ts));
    }

    sql.push_str(" ORDER BY fetched_at DESC LIMIT ?");
    params_vec.push(rusqlite::types::Value::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let item_iter = stmt.query_map(rusqlite::params_from_iter(params_vec), digest_item_from_row)?;

    let mut items = Vec::new();
    for item in item_iter {
        items.push(item?);
    }
    Ok(items)
}

fn digest_item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DigestItem> {
    Ok(DigestItem {
        id: row.get(0)?,
        source: row.get(1)?,
        category: row.get(2)?,
        title: row.get(3)?,
        summary: row.get(4)?,
        thumbnail_url: row.get(5)?,
        url: row.get(6)?,
        published_at: row.get(7)?,
        fetched_at: row.get(8)?,
        seen: row.get(9)?,
        starred: row.get(10)?,
        thumbnail_resolved: row.get(11)?,
    })
}

pub fn get_latest_fetched_at(conn: &Connection) -> Result<Option<String>> {
    conn.query_row("SELECT MAX(fetched_at) FROM digest_items", [], |row| {
        row.get(0)
    })
}

pub fn toggle_starred(conn: &Connection, id: &str) -> Result<i32> {
    conn.execute(
        "UPDATE digest_items SET starred = CASE WHEN starred = 1 THEN 0 ELSE 1 END WHERE id = ?",
        params![id],
    )?;

    conn.query_row(
        "SELECT starred FROM digest_items WHERE id = ?",
        params![id],
        |row| row.get(0),
    )
}

pub fn get_sources(conn: &Connection) -> Result<Vec<Source>> {
    let mut stmt = conn.prepare("SELECT id, name, enabled FROM sources ORDER BY rowid")?;
    let source_iter = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            enabled: row.get(2)?,
        })
    })?;

    let mut sources = Vec::new();
    for source in source_iter {
        sources.push(source?);
    }
    Ok(sources)
}

pub fn get_enabled_source_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM sources WHERE enabled = 1")?;
    let source_iter = stmt.query_map([], |row| row.get(0))?;

    let mut sources = Vec::new();
    for source in source_iter {
        sources.push(source?);
    }
    Ok(sources)
}

pub fn toggle_source(conn: &Connection, id: &str) -> Result<Source> {
    conn.execute(
        "UPDATE sources SET enabled = CASE WHEN enabled = 1 THEN 0 ELSE 1 END WHERE id = ?",
        params![id],
    )?;

    conn.query_row(
        "SELECT id, name, enabled FROM sources WHERE id = ?",
        params![id],
        |row| {
            Ok(Source {
                id: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get(2)?,
            })
        },
    )
}

pub fn mark_seen(conn: &Connection, ids: &[String]) -> Result<()> {
    // We update multiple items. For many items a loop is fine.
    let mut stmt = conn.prepare("UPDATE digest_items SET seen = 1 WHERE id = ?")?;
    for id in ids {
        stmt.execute(params![id])?;
    }
    Ok(())
}

pub fn clear_old_items(conn: &Connection, days: u32) -> Result<()> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
    let cutoff_str = cutoff.to_rfc3339();

    conn.execute(
        "DELETE FROM digest_items WHERE published_at < ? AND starred = 0",
        params![cutoff_str],
    )?;
    Ok(())
}
