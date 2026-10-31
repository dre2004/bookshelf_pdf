//! SQLite persistence: library entries, reading progress, covers and
//! application settings. One connection guarded by a mutex — writes are
//! small and infrequent, so contention is a non-issue.

use std::{collections::HashMap, path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: i64,
    pub page: i32,
    pub offset: f64,
    pub title: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    pub id: i64,
    pub page: i32,
    pub start_char: i64,
    pub end_char: i64,
    pub text: String,
    pub color: String,
    pub note: String,
    pub created_at: i64,
}

/// Input for creating a highlight; deserialized from the frontend command.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewHighlight {
    pub book_id: i64,
    pub page: i32,
    pub start_char: i64,
    pub end_char: i64,
    pub text: String,
    pub color: String,
    pub note: String,
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
        // Verification instances keep their data separate so automated
        // testing can never touch a real library or reading progress.
        let dir = if crate::is_verify_instance() {
            dir.join("verify-test")
        } else {
            dir
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create app data dir: {e}"))?;
        let db_path = dir.join("bookshelf.db");
        let conn = Self::connect(&db_path)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn connect(path: &Path) -> Result<Connection, String> {
        let conn = Connection::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        Self::init_schema(&conn)?;
        Ok(conn)
    }

    fn init_schema(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
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
             );
             CREATE TABLE IF NOT EXISTS bookmarks (
                 id          INTEGER PRIMARY KEY,
                 book_id     INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
                 page        INTEGER NOT NULL,
                 offset      REAL NOT NULL DEFAULT 0,
                 title       TEXT NOT NULL,
                 created_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS highlights (
                 id          INTEGER PRIMARY KEY,
                 book_id     INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
                 page        INTEGER NOT NULL,
                 start_char  INTEGER NOT NULL,
                 end_char    INTEGER NOT NULL,
                 text        TEXT NOT NULL,
                 color       TEXT NOT NULL DEFAULT 'yellow',
                 note        TEXT NOT NULL DEFAULT '',
                 created_at  INTEGER NOT NULL
             );",
        )
        .map_err(|e| format!("migrate: {e}"))?;
        Ok(())
    }

    pub fn add_or_touch_book(
        &self,
        path: &str,
        title: &str,
        page_count: u32,
    ) -> Result<i64, String> {
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
        // last_insert_rowid() is wrong on the conflict path (it returns the
        // rowid of whatever insert ran last on this connection), which handed
        // back invalid book ids for already-registered books. Read the row.
        conn.query_row(
            "SELECT id FROM books WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
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

    pub fn update_progress(
        &self,
        book_id: i64,
        last_page: i32,
        last_offset: f64,
    ) -> Result<(), String> {
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
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn settings_get_all(&self) -> Result<HashMap<String, String>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT key, value FROM settings")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|e| e.to_string())
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

    // ---- named bookmarks ----

    pub fn add_bookmark(
        &self,
        book_id: i64,
        page: i32,
        offset: f64,
        title: &str,
    ) -> Result<Bookmark, String> {
        let conn = self.conn.lock().unwrap();
        let created_at = now_ms();
        conn.execute(
            "INSERT INTO bookmarks (book_id, page, offset, title, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![book_id, page, offset, title, created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(Bookmark {
            id: conn.last_insert_rowid(),
            page,
            offset,
            title: title.to_string(),
            created_at,
        })
    }

    pub fn remove_bookmark(&self, bookmark_id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM bookmarks WHERE id = ?1", params![bookmark_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_bookmarks(&self, book_id: i64) -> Result<Vec<Bookmark>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, page, offset, title, created_at
                 FROM bookmarks WHERE book_id = ?1
                 ORDER BY page, created_at",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![book_id], |row| {
                Ok(Bookmark {
                    id: row.get(0)?,
                    page: row.get(1)?,
                    offset: row.get(2)?,
                    title: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    // ---- highlights & notes ----

    pub fn add_highlight(&self, entry: &NewHighlight) -> Result<Highlight, String> {
        let conn = self.conn.lock().unwrap();
        let created_at = now_ms();
        conn.execute(
            "INSERT INTO highlights (book_id, page, start_char, end_char, text, color, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.book_id,
                entry.page,
                entry.start_char,
                entry.end_char,
                entry.text,
                entry.color,
                entry.note,
                created_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(Highlight {
            id: conn.last_insert_rowid(),
            page: entry.page,
            start_char: entry.start_char,
            end_char: entry.end_char,
            text: entry.text.clone(),
            color: entry.color.clone(),
            note: entry.note.clone(),
            created_at,
        })
    }

    pub fn update_highlight(
        &self,
        highlight_id: i64,
        color: Option<&str>,
        note: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE highlights SET
                     color = COALESCE(?1, color),
                     note = COALESCE(?2, note)
                 WHERE id = ?3",
                params![color, note, highlight_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn remove_highlight(&self, highlight_id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM highlights WHERE id = ?1",
                params![highlight_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_highlights(&self, book_id: i64) -> Result<Vec<Highlight>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, page, start_char, end_char, text, color, note, created_at
                 FROM highlights WHERE book_id = ?1
                 ORDER BY page, start_char",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![book_id], |row| {
                Ok(Highlight {
                    id: row.get(0)?,
                    page: row.get(1)?,
                    start_char: row.get(2)?,
                    end_char: row.get(3)?,
                    text: row.get(4)?,
                    color: row.get(5)?,
                    note: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Db {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        Db::init_schema(&conn).expect("init schema");
        Db {
            conn: Mutex::new(conn),
        }
    }

    #[test]
    fn settings_roundtrip_and_overwrite() {
        let db = test_db();
        assert!(db.settings_get_all().unwrap().is_empty());

        db.settings_set("theme", "\"night\"").unwrap();
        db.settings_set("theme", "\"sepia\"").unwrap();
        db.settings_set("readMode", "\"scroll\"").unwrap();

        let all = db.settings_get_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("theme").unwrap(), "\"sepia\"");
        assert_eq!(all.get("readMode").unwrap(), "\"scroll\"");
    }

    #[test]
    fn add_or_touch_updates_existing_book() {
        let db = test_db();
        let id_first = db.add_or_touch_book("a.pdf", "A", 10).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let id_again = db.add_or_touch_book("a.pdf", "A (edited)", 42).unwrap();
        let id_other = db.add_or_touch_book("b.pdf", "B", 5).unwrap();

        assert_eq!(id_first, id_again);
        assert_ne!(id_first, id_other);

        let books = db.list_books().unwrap();
        let a = books.iter().find(|b| b.path == "a.pdf").unwrap();
        assert_eq!(a.title, "A (edited)");
        assert_eq!(a.page_count, 42);
    }

    #[test]
    fn add_or_touch_returns_stable_id_despite_interleaved_inserts() {
        // Reopening a registered book takes the ON CONFLICT update path, so
        // last_insert_rowid() (the old implementation) would return the
        // rowid of the latest insert on the connection — e.g. a settings
        // row — which broke bookmark foreign keys and silently dropped
        // progress saves for existing books.
        let db = test_db();
        let id = db.add_or_touch_book("a.pdf", "A", 10).unwrap();
        db.settings_set("theme", "\"night\"").unwrap();
        db.settings_set("readMode", "\"scroll\"").unwrap();
        let id_again = db.add_or_touch_book("a.pdf", "A", 10).unwrap();
        assert_eq!(id, id_again);

        // And the id is usable for child rows.
        db.add_bookmark(id_again, 3, 0.0, "works now").unwrap();
        assert_eq!(db.list_bookmarks(id_again).unwrap().len(), 1);
    }

    #[test]
    fn progress_save_and_restore() {
        let db = test_db();
        let id = db.add_or_touch_book("book.pdf", "Book", 200).unwrap();

        assert_eq!(db.book_state(id).expect("state exists").last_page, 0);

        db.update_progress(id, 87, 0.42).unwrap();
        let state = db.book_state(id).expect("state exists");
        assert_eq!(state.last_page, 87);
        assert!((state.last_offset - 0.42).abs() < 1e-9);

        let books = db.list_books().unwrap();
        assert_eq!(books[0].progress_pct, 44); // 87/200 rounded
    }

    #[test]
    fn remove_book_deletes_only_target() {
        let db = test_db();
        let a = db.add_or_touch_book("a.pdf", "A", 10).unwrap();
        let b = db.add_or_touch_book("b.pdf", "B", 10).unwrap();
        db.remove_book(a).unwrap();

        let books = db.list_books().unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].book_id, b);
    }

    #[test]
    fn list_books_orders_by_recently_opened() {
        let db = test_db();
        let a = db.add_or_touch_book("a.pdf", "A", 10).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = db.add_or_touch_book("b.pdf", "B", 10).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        db.update_progress(a, 1, 0.0).unwrap(); // touches last_opened_at

        let books = db.list_books().unwrap();
        assert_eq!(books[0].book_id, a);
        assert_eq!(books[1].book_id, b);
    }

    #[test]
    fn cover_roundtrip() {
        let db = test_db();
        let id = db.add_or_touch_book("a.pdf", "A", 10).unwrap();

        assert!(db.cover(id).is_none());
        assert!(!db.list_books().unwrap()[0].has_cover);

        db.set_cover(id, vec![1, 2, 3]).unwrap();
        assert_eq!(db.cover(id).unwrap(), vec![1, 2, 3]);
        assert!(db.list_books().unwrap()[0].has_cover);
    }

    #[test]
    fn bookmarks_crud_and_ordering() {
        let db = test_db();
        let book = db.add_or_touch_book("a.pdf", "A", 100).unwrap();

        db.add_bookmark(book, 42, 0.5, "Halfway").unwrap();
        db.add_bookmark(book, 3, 0.0, "Start").unwrap();
        db.add_bookmark(book, 42, 0.9, "Also halfway").unwrap();

        let bookmarks = db.list_bookmarks(book).unwrap();
        assert_eq!(bookmarks.len(), 3);
        assert_eq!(bookmarks[0].page, 3); // ordered by page
        assert_eq!(bookmarks[1].title, "Halfway"); // then by creation

        db.remove_bookmark(bookmarks[0].id).unwrap();
        assert_eq!(db.list_bookmarks(book).unwrap().len(), 2);

        // Removing the book cascades to its bookmarks.
        db.remove_book(book).unwrap();
        assert!(db.list_bookmarks(book).unwrap().is_empty());
    }

    #[test]
    fn highlights_crud_and_partial_updates() {
        let db = test_db();
        let book = db.add_or_touch_book("a.pdf", "A", 100).unwrap();

        let hl = db
            .add_highlight(&NewHighlight {
                book_id: book,
                page: 7,
                start_char: 120,
                end_char: 145,
                text: "important passage".into(),
                color: "yellow".into(),
                note: String::new(),
            })
            .unwrap();
        assert_eq!(hl.page, 7);
        assert_eq!(hl.color, "yellow");

        db.update_highlight(hl.id, Some("green"), Some("my note"))
            .unwrap();
        db.add_highlight(&NewHighlight {
            book_id: book,
            page: 9,
            start_char: 10,
            end_char: 20,
            text: "another".into(),
            color: "blue".into(),
            note: String::new(),
        })
        .unwrap();

        let highlights = db.list_highlights(book).unwrap();
        assert_eq!(highlights.len(), 2);
        let updated = highlights.iter().find(|h| h.id == hl.id).unwrap();
        assert_eq!(updated.color, "green");
        assert_eq!(updated.note, "my note");

        // Updating only one field leaves the other intact.
        db.update_highlight(hl.id, Some("red"), None).unwrap();
        let updated = db.list_highlights(book).unwrap()[0].clone();
        assert_eq!(updated.color, "red");
        assert_eq!(updated.note, "my note");

        db.remove_highlight(hl.id).unwrap();
        assert_eq!(db.list_highlights(book).unwrap().len(), 1);
    }
}
