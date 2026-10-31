mod commands;
mod db;
mod pdfium_engine;

use std::sync::Arc;

use db::Db;
use pdfium_engine::Engine;
use tauri::Manager;

/// Serves rendered pages and stored cover thumbnails over a custom `pdfimg`
/// URI scheme so the webview can load them as plain image URLs — the browser
/// handles decoding, caching and cancellation, and no image bytes travel
/// through the IPC bridge.
///
/// URL shapes:
///   pdfimg://localhost/page/{doc_id}/{page_index}/{width_px}
///   pdfimg://localhost/cover/{book_id}
/// (`http://pdfimg.localhost/...` on Windows, where WebView2 maps custom
/// schemes onto http subdomains.)
enum ProtocolResponse {
    Png(Vec<u8>),
    NotFound(String),
}

fn handle_page_request(engine: &Engine, db: &Db, path: &str) -> ProtocolResponse {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        ["page", doc_id, page_index, width] => {
            let parse = || -> Result<Vec<u8>, String> {
                let doc_id: u32 = doc_id.parse().map_err(|_| "invalid document id")?;
                let page_index: i32 = page_index.parse().map_err(|_| "invalid page index")?;
                let width: u32 = width.parse().map_err(|_| "invalid width")?;
                engine
                    .render_page_png(doc_id, page_index, width)
                    .map(|png| (*png).clone())
                    .map_err(|e| format!("{e:?}"))
            };
            match parse() {
                Ok(png) => ProtocolResponse::Png(png),
                Err(reason) => ProtocolResponse::NotFound(reason),
            }
        }
        ["cover", book_id] => match book_id.parse::<i64>() {
            Ok(id) => match db.cover(id) {
                Some(png) => ProtocolResponse::Png(png),
                None => ProtocolResponse::NotFound("no cover".into()),
            },
            Err(_) => ProtocolResponse::NotFound("invalid book id".into()),
        },
        _ => ProtocolResponse::NotFound(format!("unrecognized request path: {path}")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let engine = Engine::new(app.handle()).map_err(std::io::Error::other)?;
            let db = Db::open(app.handle()).map_err(std::io::Error::other)?;
            app.manage(Arc::new(engine));
            app.manage(Arc::new(db));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_document,
            commands::close_document,
            commands::get_page_dims,
            commands::get_page_text,
            commands::search_document,
            commands::get_outline,
            commands::bookmarks_add,
            commands::bookmarks_remove,
            commands::bookmarks_list,
            commands::highlights_add,
            commands::highlights_update,
            commands::highlights_remove,
            commands::highlights_list,
            commands::book_register,
            commands::book_state,
            commands::book_progress_save,
            commands::library_list,
            commands::library_remove,
            commands::settings_all,
            commands::settings_set
        ])
        .register_asynchronous_uri_scheme_protocol("pdfimg", |ctx, request, responder| {
            let engine = ctx.app_handle().state::<Arc<Engine>>().inner().clone();
            let db = ctx.app_handle().state::<Arc<Db>>().inner().clone();
            let path = request.uri().path().to_string();
            std::thread::spawn(move || {
                let response = match handle_page_request(&engine, &db, &path) {
                    ProtocolResponse::Png(png) => tauri::http::Response::builder()
                        .header("Content-Type", "image/png")
                        .header("Cache-Control", "private, max-age=86400")
                        .body(png)
                        .unwrap(),
                    ProtocolResponse::NotFound(reason) => tauri::http::Response::builder()
                        .status(tauri::http::StatusCode::NOT_FOUND)
                        .header("Content-Type", "text/plain")
                        .body(reason.into_bytes())
                        .unwrap(),
                };
                responder.respond(response);
            });
        })
        .run(tauri::generate_context!())
        .expect("error while running bookshelf_pdf");
}
