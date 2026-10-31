//! SQLite persistence: library entries, reading progress, covers and
//! application settings. One connection guarded by a mutex — writes are
//! small and infrequent, so contention is a non-issue.

use std::{collections::HashMap, path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::Manager;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub book_id: i64,
    pub path: String,
    pub title: String,
    pub page_count: u32,
    pub last_page: i32,
    /// Position within the last page, 0..1 (used to restore scroll offsets).
    pub last_offset: f64,
    pub progress_pct: u32,
    pub last_opened_at: i64,
    pub has_cover: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookState {
    pub last_page: i32,
    pub last_offset: f64,
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(app: &tauri::AppHandle) -> Result<Self, String> {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("cannot resolve app data dir: {e}"))?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create app data dir: {e}"))?;
        let db_path = dir.join("bookshelf.db");
        let conn = Self::connect(&db_path)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn connect(path: &Path) -> Result<Connection, String> {
        let conn = Connection::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS books (
                 id              INTEGER PRIMARY KEY,
                 path            TEXT NOT NULL UNIQUE,
                 title           TEXT NOT NULL,
                 page_count      INTEGER NOT NULL,
                 cover           BLOB,
                 last_page       INTEGER NOT NULL DEFAULT 0,
                 last_offset     REAL NOT NULL DEFAULT 0,
                 last_opened_at  INTEGER NOT NULL DEFAULT 0,
                 created_at      INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .map_err(|e| format!("migrate: {e}"))?;
        Ok(conn)
    }

    pub fn add_or_touch_book(&self, path: &str, title: &str, page_count: u32) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        let now = now_ms();
        conn.execute(
            "INSERT INTO books (path, title, page_count, last_opened_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(path) DO UPDATE SET
                 last_opened_at = ?4,
                 page_count = ?3,
                 title = ?2",
            params![path, title, page_count as i64, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub fn set_cover(&self, book_id: i64, png: Vec<u8>) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE books SET cover = ?1 WHERE id = ?2",
                params![png, book_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn cover(&self, book_id: i64) -> Option<Vec<u8>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT cover FROM books WHERE id = ?1 AND cover IS NOT NULL",
                params![book_id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn update_progress(&self, book_id: i64, last_page: i32, last_offset: f64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE books SET last_page = ?1, last_offset = ?2, last_opened_at = ?3
                 WHERE id = ?4",
                params![last_page, last_offset, now_ms(), book_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn book_state(&self, book_id: i64) -> Option<BookState> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT last_page, last_offset FROM books WHERE id = ?1",
                params![book_id],
                |row| {
                    Ok(BookState {
                        last_page: row.get(0)?,
                        last_offset: row.get(1)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn remove_book(&self, book_id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM books WHERE id = ?1", params![book_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_books(&self) -> Result<Vec<BookSummary>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, path, title, page_count, last_page, last_offset, last_opened_at,
                        cover IS NOT NULL
                 FROM books
                 ORDER BY last_opened_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let page_count: i64 = row.get(3)?;
                let last_page: i32 = row.get(4)?;
                let progress_pct = if page_count > 0 {
                    (((last_page.max(0) as f64) / page_count as f64) * 100.0).round() as u32
                } else {
                    0
                };
                Ok(BookSummary {
                    book_id: row.get(0)?,
                    path: row.get(1)?,
                    title: row.get(2)?,
                    page_count: page_count as u32,
                    last_page,
                    last_offset: row.get(5)?,
                    progress_pct: progress_pct.min(100),
                    last_opened_at: row.get(6)?,
                    has_cover: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn settings_get_all(&self) -> Result<HashMap<String, String>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT key, value FROM settings").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<HashMap<_, _>, _>>().map_err(|e| e.to_string())
    }

    pub fn settings_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
