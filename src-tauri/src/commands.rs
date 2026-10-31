//! Tauri commands exposed to the frontend.

use std::{collections::HashMap, sync::Arc};

use crate::db::{BookState, BookSummary, Bookmark, Db, Highlight, NewHighlight};
use crate::pdfium_engine::{
    ApiError, DocumentInfo, Engine, OutlineNode, PageInfo, PageTextData, SearchMatch,
};

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

#[tauri::command]
pub fn get_page_text(
    engine: tauri::State<'_, Arc<Engine>>,
    doc_id: u32,
    page: i32,
) -> Result<PageTextData, ApiError> {
    engine.page_text(doc_id, page)
}

#[tauri::command]
pub fn search_document(
    engine: tauri::State<'_, Arc<Engine>>,
    doc_id: u32,
    query: String,
    case_sensitive: bool,
) -> Result<Vec<SearchMatch>, ApiError> {
    engine.search_document(doc_id, &query, case_sensitive, 2000)
}

#[tauri::command]
pub fn get_outline(
    engine: tauri::State<'_, Arc<Engine>>,
    doc_id: u32,
) -> Result<Vec<OutlineNode>, ApiError> {
    engine.outline(doc_id)
}

#[tauri::command]
pub fn bookmarks_add(
    db: tauri::State<'_, Arc<Db>>,
    book_id: i64,
    page: i32,
    offset: f64,
    title: String,
) -> Result<Bookmark, String> {
    db.add_bookmark(book_id, page, offset, &title)
}

#[tauri::command]
pub fn bookmarks_remove(db: tauri::State<'_, Arc<Db>>, bookmark_id: i64) -> Result<(), String> {
    db.remove_bookmark(bookmark_id)
}

#[tauri::command]
pub fn bookmarks_list(
    db: tauri::State<'_, Arc<Db>>,
    book_id: i64,
) -> Result<Vec<Bookmark>, String> {
    db.list_bookmarks(book_id)
}

#[tauri::command]
pub fn highlights_add(
    db: tauri::State<'_, Arc<Db>>,
    entry: NewHighlight,
) -> Result<Highlight, String> {
    db.add_highlight(&entry)
}

#[tauri::command]
pub fn highlights_update(
    db: tauri::State<'_, Arc<Db>>,
    highlight_id: i64,
    color: Option<String>,
    note: Option<String>,
) -> Result<(), String> {
    db.update_highlight(highlight_id, color.as_deref(), note.as_deref())
}

#[tauri::command]
pub fn highlights_remove(db: tauri::State<'_, Arc<Db>>, highlight_id: i64) -> Result<(), String> {
    db.remove_highlight(highlight_id)
}

#[tauri::command]
pub fn highlights_list(
    db: tauri::State<'_, Arc<Db>>,
    book_id: i64,
) -> Result<Vec<Highlight>, String> {
    db.list_highlights(book_id)
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
pub fn book_state(
    db: tauri::State<'_, Arc<Db>>,
    book_id: i64,
) -> Result<Option<BookState>, String> {
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

/// PDF path passed on the command line (e.g. double-clicking a file while
/// the app is closed), if any.
#[tauri::command]
pub fn get_startup_file() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|arg| arg.to_lowercase().ends_with(".pdf"))
}
