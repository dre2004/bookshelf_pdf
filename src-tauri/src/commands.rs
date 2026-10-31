//! Tauri commands exposed to the frontend.

use std::{collections::HashMap, sync::Arc};

use crate::db::{BookState, BookSummary, Db};
use crate::pdfium_engine::{ApiError, DocumentInfo, Engine, PageInfo};

#[tauri::command]
pub fn open_document(
    engine: tauri::State<'_, Arc<Engine>>,
    path: String,
    password: Option<String>,
) -> Result<DocumentInfo, ApiError> {
    engine.open_document(&path, password.as_deref())
}

#[tauri::command]
pub fn close_document(engine: tauri::State<'_, Arc<Engine>>, doc_id: u32) -> Result<(), ApiError> {
    engine.close_document(doc_id);
    Ok(())
}

#[tauri::command]
pub fn get_page_dims(
    engine: tauri::State<'_, Arc<Engine>>,
    doc_id: u32,
    from: i32,
    to: i32,
) -> Result<Vec<PageInfo>, ApiError> {
    engine.get_page_dims(doc_id, from, to)
}

/// Registers a just-opened book in the library (or touches its last-opened
/// time) and refreshes its cover thumbnail in the background.
#[tauri::command]
pub fn book_register(
    engine: tauri::State<'_, Arc<Engine>>,
    db: tauri::State<'_, Arc<Db>>,
    path: String,
    title: String,
    page_count: u32,
    doc_id: u32,
) -> Result<i64, String> {
    let book_id = db.add_or_touch_book(&path, &title, page_count)?;

    let engine = engine.inner().clone();
    let db = db.inner().clone();
    std::thread::spawn(move || {
        if let Ok(png) = engine.render_page_png(doc_id, 0, 320) {
            let _ = db.set_cover(book_id, (*png).clone());
        }
    });
    Ok(book_id)
}

#[tauri::command]
pub fn book_state(db: tauri::State<'_, Arc<Db>>, book_id: i64) -> Result<Option<BookState>, String> {
    Ok(db.book_state(book_id))
}

#[tauri::command]
pub fn book_progress_save(
    db: tauri::State<'_, Arc<Db>>,
    book_id: i64,
    page: i32,
    offset: f64,
) -> Result<(), String> {
    db.update_progress(book_id, page, offset.clamp(0.0, 1.0))
}

#[tauri::command]
pub fn library_list(db: tauri::State<'_, Arc<Db>>) -> Result<Vec<BookSummary>, String> {
    db.list_books()
}

#[tauri::command]
pub fn library_remove(db: tauri::State<'_, Arc<Db>>, book_id: i64) -> Result<(), String> {
    db.remove_book(book_id)
}

#[tauri::command]
pub fn settings_all(db: tauri::State<'_, Arc<Db>>) -> Result<HashMap<String, String>, String> {
    db.settings_get_all()
}

#[tauri::command]
pub fn settings_set(
    db: tauri::State<'_, Arc<Db>>,
    key: String,
    value: String,
) -> Result<(), String> {
    db.settings_set(&key, &value)
}
