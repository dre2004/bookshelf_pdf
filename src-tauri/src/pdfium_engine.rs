//! PDFium engine: owns the PDFium library instance, the set of open documents,
//! and a byte-bounded cache of rendered page PNGs.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
};

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use pdfium_render::prelude::*;
use serde::Serialize;
use tauri::Manager;

#[cfg(windows)]
pub const PDFIUM_LIBRARY_FILENAME: &str = "pdfium.dll";
#[cfg(target_os = "macos")]
pub const PDFIUM_LIBRARY_FILENAME: &str = "libpdfium.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
pub const PDFIUM_LIBRARY_FILENAME: &str = "libpdfium.so";

/// Page dimensions returned eagerly at open time; the rest are fetched
/// lazily with `get_page_dims` so thousand-page books still open instantly.
pub const EAGER_PAGE_DIMS: u32 = 200;

#[derive(Debug, Clone, Serialize)]
pub struct PageInfo {
    /// Width in PDF points.
    pub width: f64,
    /// Height in PDF points.
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub doc_id: u32,
    pub page_count: u32,
    pub pages: Vec<PageInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ApiError {
    PasswordRequired,
    PasswordIncorrect,
    UnknownDocument,
    UnknownPage,
    OpenFailed { reason: String },
    RenderFailed { reason: String },
}

fn pdfium_error_reason(err: &PdfiumError) -> String {
    match err {
        PdfiumError::PdfiumLibraryInternalError(e) => format!("pdfium error: {e:?}"),
        other => other.to_string(),
    }
}

/// The `Pdfium` handle is intentionally leaked: it must outlive every
/// `PdfDocument<'static>` in the registry, and `pdfium-render` keeps the
/// library bindings in a process-global anyway. Under the crate's default
/// `thread_safe` feature every bindings call serializes on an internal lock,
/// which is what makes the documents safely shareable here.
pub struct Engine {
    pdfium: &'static Pdfium,
    docs: Mutex<HashMap<u32, PdfDocument<'static>>>,
    next_doc_id: AtomicU32,
    render_cache: Mutex<RenderCache>,
}

impl Engine {
    pub fn new(app: &tauri::AppHandle) -> Result<Self, String> {
        let library_path = Self::resolve_library_path(app)?;
        let bindings = Pdfium::bind_to_library(&library_path)
            .map_err(|e| format!("failed to load PDFium from {}: {e}", library_path.display()))?;
        let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
        Ok(Self {
            pdfium,
            docs: Mutex::new(HashMap::new()),
            next_doc_id: AtomicU32::new(1),
            render_cache: Mutex::new(RenderCache::new()),
        })
    }

    fn resolve_library_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(env_path) = std::env::var("BOOKSHELF_PDFIUM_PATH") {
            candidates.push(PathBuf::from(env_path));
        }
        if let Ok(resource_dir) = app.path().resource_dir() {
            candidates.push(resource_dir.join("pdfium").join(PDFIUM_LIBRARY_FILENAME));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(PDFIUM_LIBRARY_FILENAME));
                candidates.push(dir.join("pdfium").join(PDFIUM_LIBRARY_FILENAME));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("resources").join("pdfium").join(PDFIUM_LIBRARY_FILENAME));
            candidates.push(
                cwd.join("src-tauri")
                    .join("resources")
                    .join("pdfium")
                    .join(PDFIUM_LIBRARY_FILENAME),
            );
        }
        for candidate in &candidates {
            if candidate.is_file() {
                return Ok(candidate.clone());
            }
        }
        Err(format!(
            "PDFium library not found. Run `npm run fetch:pdfium` first, or set BOOKSHELF_PDFIUM_PATH \
             to point at {PDFIUM_LIBRARY_FILENAME}. Tried: {}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    pub fn open_document(
        &self,
        path: &str,
        password: Option<&str>,
    ) -> Result<DocumentInfo, ApiError> {
        let password = password.filter(|p| !p.is_empty());
        let doc = self
            .pdfium
            .load_pdf_from_file(path, password)
            .map_err(|e| match e {
                PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
                    if password.is_some() {
                        ApiError::PasswordIncorrect
                    } else {
                        ApiError::PasswordRequired
                    }
                }
                other => ApiError::OpenFailed {
                    reason: pdfium_error_reason(&other),
                },
            })?;

        let page_count = doc.pages().len() as u32;
        let eager = EAGER_PAGE_DIMS.min(page_count);
        let pages = collect_dims(&doc, 0, eager as PdfPageIndex);
        let doc_id = self.next_doc_id.fetch_add(1, Ordering::Relaxed);
        self.docs.lock().unwrap().insert(doc_id, doc);
        Ok(DocumentInfo {
            doc_id,
            page_count,
            pages,
        })
    }

    pub fn close_document(&self, doc_id: u32) {
        self.docs.lock().unwrap().remove(&doc_id);
        self.render_cache.lock().unwrap().retain_doc(doc_id);
    }

    pub fn get_page_dims(
        &self,
        doc_id: u32,
        from: PdfPageIndex,
        to: PdfPageIndex,
    ) -> Result<Vec<PageInfo>, ApiError> {
        let docs = self.docs.lock().unwrap();
        let doc = docs.get(&doc_id).ok_or(ApiError::UnknownDocument)?;
        let page_count = doc.pages().len();
        let from = from.max(0).min(page_count);
        let to = to.max(from).min(page_count);
        Ok(collect_dims(doc, from, to))
    }

    pub fn render_page_png(
        &self,
        doc_id: u32,
        page_index: PdfPageIndex,
        target_width: u32,
    ) -> Result<Arc<Vec<u8>>, ApiError> {
        let target_width = target_width.clamp(100, 6000);
        let cache_key = (doc_id, page_index, target_width);
        if let Some(png) = self.render_cache.lock().unwrap().get(&cache_key) {
            return Ok(png);
        }

        // Render inside the document lock, encode outside it so other
        // documents can render while PNG compression runs.
        let image = {
            let docs = self.docs.lock().unwrap();
            let doc = docs.get(&doc_id).ok_or(ApiError::UnknownDocument)?;
            let pages = doc.pages();
            if page_index < 0 || page_index >= pages.len() {
                return Err(ApiError::UnknownPage);
            }
            let page = pages.get(page_index).map_err(|e| ApiError::RenderFailed {
                reason: pdfium_error_reason(&e),
            })?;
            let config = PdfRenderConfig::new()
                .set_target_width(target_width as Pixels)
                .set_format(PdfBitmapFormat::BGRA)
                .set_clear_color(PdfColor::WHITE);
            let bitmap = page
                .render_with_config(&config)
                .map_err(|e| ApiError::RenderFailed {
                    reason: pdfium_error_reason(&e),
                })?;
            bitmap
                .as_image()
                .map_err(|e| ApiError::RenderFailed {
                    reason: pdfium_error_reason(&e),
                })?
        };

        // Fast deflate keeps encode time small; page images are cache-only
        // artifacts, so file size matters far less than latency here.
        let mut png = Vec::new();
        let rgb = image.to_rgb8();
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                ExtendedColorType::Rgb8,
            )
            .map_err(|e| ApiError::RenderFailed {
                reason: format!("png encode: {e}"),
            })?;
        let png = Arc::new(png);
        self.render_cache
            .lock()
            .unwrap()
            .insert(cache_key, png.clone());
        Ok(png)
    }
}

fn collect_dims(doc: &PdfDocument<'_>, from: PdfPageIndex, to: PdfPageIndex) -> Vec<PageInfo> {
    let pages = doc.pages();
    let mut dims = Vec::with_capacity(to.saturating_sub(from).max(0) as usize);
    for i in from..to {
        match pages.get(i).ok() {
            Some(page) => dims.push(PageInfo {
                width: page.width().value as f64,
                height: page.height().value as f64,
            }),
            // A4 fallback for pages whose box can't be read.
            None => dims.push(PageInfo {
                width: 595.0,
                height: 842.0,
            }),
        }
    }
    dims
}

/// LRU cache of encoded page renders, bounded by total bytes rather than
/// entry count so deep zoom on large pages cannot balloon memory.
struct RenderCache {
    entries: HashMap<CacheKey, (Arc<Vec<u8>>, u64)>,
    total_bytes: usize,
    tick: u64,
}

type CacheKey = (u32, PdfPageIndex, u32);

/// Soft cap on cached rendered PNGs (~192 MB).
const RENDER_CACHE_MAX_BYTES: usize = 192 * 1024 * 1024;

impl RenderCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
            tick: 0,
        }
    }

    fn get(&mut self, key: &CacheKey) -> Option<Arc<Vec<u8>>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(data, last_used)| {
            *last_used = tick;
            data.clone()
        })
    }

    fn insert(&mut self, key: CacheKey, data: Arc<Vec<u8>>) {
        self.tick += 1;
        let tick = self.tick;
        let len = data.len();
        if let Some((old, _)) = self.entries.insert(key, (data, tick)) {
            self.total_bytes = self.total_bytes.saturating_sub(old.len());
        }
        self.total_bytes += len;
        while self.total_bytes > RENDER_CACHE_MAX_BYTES && self.entries.len() > 1 {
            let evict_key = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(k, _)| *k);
            match evict_key.and_then(|k| self.entries.remove(&k)) {
                Some((data, _)) => self.total_bytes = self.total_bytes.saturating_sub(data.len()),
                None => break,
            }
        }
    }

    fn retain_doc(&mut self, doc_id: u32) {
        self.entries.retain(|(doc, _, _), (data, _)| {
            if *doc == doc_id {
                self.total_bytes = self.total_bytes.saturating_sub(data.len());
                false
            } else {
                true
            }
        });
    }
}
