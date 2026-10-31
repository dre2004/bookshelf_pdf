//! PDFium engine: owns the PDFium library instance, the set of open documents,
//! and a byte-bounded cache of rendered page PNGs.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
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

/// Character-level text geometry for one page. `chars[i]` corresponds to
/// `boxes[i*4..i*4+4]` = (x, y, width, height) in PDF points, origin at the
/// page's bottom-left corner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTextData {
    pub chars: String,
    pub boxes: Vec<f32>,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub page: PdfPageIndex,
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineNode {
    pub title: String,
    /// Target page index, or -1 when the bookmark has no page destination.
    pub page: PdfPageIndex,
    pub children: Vec<OutlineNode>,
}

/// Collapses `\r\n` pairs into single spaces so queries match across line
/// breaks; returns the normalized text plus a map from normalized index to
/// original character index.
fn normalize_for_search(chars: &[char]) -> (Vec<char>, Vec<usize>) {
    let mut normalized = Vec::with_capacity(chars.len());
    let mut index_map = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' {
            normalized.push(' ');
            index_map.push(i);
            i += 2;
        } else {
            normalized.push(chars[i]);
            index_map.push(i);
            i += 1;
        }
    }
    (normalized, index_map)
}

fn chars_equal(a: char, b: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        a == b
    } else {
        a.to_lowercase().eq(b.to_lowercase())
    }
}

fn find_matches(haystack: &[char], needle: &[char], case_sensitive: bool) -> Vec<(usize, usize)> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    'outer: for start in 0..=(haystack.len() - needle.len()) {
        for (offset, expected) in needle.iter().enumerate() {
            if !chars_equal(haystack[start + offset], *expected, case_sensitive) {
                continue 'outer;
            }
        }
        matches.push((start, start + needle.len()));
    }
    matches
}

fn outline_node(bookmark: &PdfBookmark<'_>, depth: usize) -> OutlineNode {
    let page = bookmark
        .destination()
        .and_then(|dest| dest.page_index().ok())
        .unwrap_or(-1);
    let mut children = Vec::new();
    if depth < 8 {
        let mut child = bookmark.first_child();
        while let Some(node) = child {
            children.push(outline_node(&node, depth + 1));
            child = node.next_sibling();
        }
    }
    OutlineNode {
        title: bookmark.title().unwrap_or_else(|| "(untitled)".into()),
        page,
        children,
    }
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
        Self::with_library_path(&Self::resolve_library_path(app)?)
    }

    /// Engine bound to an explicit PDFium library path — the testable core
    /// of `new`, which differs only in how the library is located.
    pub fn with_library_path(library_path: &Path) -> Result<Self, String> {
        let bindings = Pdfium::bind_to_library(library_path)
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
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        {
            candidates.push(dir.join(PDFIUM_LIBRARY_FILENAME));
            candidates.push(dir.join("pdfium").join(PDFIUM_LIBRARY_FILENAME));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(
                cwd.join("resources")
                    .join("pdfium")
                    .join(PDFIUM_LIBRARY_FILENAME),
            );
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

    /// Per-character text with bounding boxes for one page — the data behind
    /// the selectable text layer, search highlighting and highlights.
    pub fn page_text(
        &self,
        doc_id: u32,
        page_index: PdfPageIndex,
    ) -> Result<PageTextData, ApiError> {
        let docs = self.docs.lock().unwrap();
        let doc = docs.get(&doc_id).ok_or(ApiError::UnknownDocument)?;
        let pages = doc.pages();
        if page_index < 0 || page_index >= pages.len() {
            return Err(ApiError::UnknownPage);
        }
        let page = pages.get(page_index).map_err(|e| ApiError::RenderFailed {
            reason: pdfium_error_reason(&e),
        })?;
        let text = page.text().map_err(|e| ApiError::RenderFailed {
            reason: pdfium_error_reason(&e),
        })?;

        let mut chars = String::with_capacity(text.len().max(0) as usize);
        let mut boxes = Vec::with_capacity(chars.capacity() * 4);
        for ch in text.chars().iter() {
            chars.push(ch.unicode_char().unwrap_or('\u{FFFD}'));
            match ch.loose_bounds() {
                Ok(rect) => {
                    boxes.push(rect.left().value);
                    boxes.push(rect.bottom().value);
                    boxes.push(rect.width().value);
                    boxes.push(rect.height().value);
                }
                Err(_) => boxes.extend([0.0, 0.0, 0.0, 0.0]),
            }
        }

        Ok(PageTextData {
            chars,
            boxes,
            width: page.width().value as f64,
            height: page.height().value as f64,
        })
    }

    /// Case-(in)sensitive substring search over every page. Matches are
    /// reported as char ranges aligned with `PageTextData::chars`, and match
    /// across line breaks (PDFium emits `\r\n`) as a single space.
    pub fn search_document(
        &self,
        doc_id: u32,
        query: &str,
        case_sensitive: bool,
        max_results: usize,
    ) -> Result<Vec<SearchMatch>, ApiError> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let query: Vec<char> = query.chars().collect();
        let docs = self.docs.lock().unwrap();
        let doc = docs.get(&doc_id).ok_or(ApiError::UnknownDocument)?;
        let pages = doc.pages();

        let mut matches = Vec::new();
        for page_index in 0..pages.len() {
            let page = match pages.get(page_index) {
                Ok(page) => page,
                Err(_) => continue,
            };
            let text = match page.text() {
                Ok(text) => text,
                Err(_) => continue,
            };
            let mut page_chars = Vec::with_capacity(text.len().max(0) as usize);
            for ch in text.chars().iter() {
                page_chars.push(ch.unicode_char().unwrap_or('\u{FFFD}'));
            }

            let (normalized, index_map) = normalize_for_search(&page_chars);
            for (norm_start, norm_end) in find_matches(&normalized, &query, case_sensitive) {
                // Map normalized positions back to real character indices.
                let start = index_map[norm_start];
                let end = if norm_end < index_map.len() {
                    index_map[norm_end]
                } else {
                    page_chars.len()
                };
                matches.push(SearchMatch {
                    page: page_index,
                    start: start as i64,
                    end: end as i64,
                });
                if matches.len() >= max_results {
                    return Ok(matches);
                }
            }
        }
        Ok(matches)
    }

    /// Outline titles whose destination is `page` — the document's own
    /// heading list for that page, used for narration heading detection.
    pub fn outline_titles_for_page(&self, doc_id: u32, page: PdfPageIndex) -> Vec<String> {
        let Ok(docs) = self.docs.try_lock() else {
            return Vec::new();
        };
        let Some(doc) = docs.get(&doc_id) else {
            return Vec::new();
        };
        let bookmarks = doc.bookmarks();
        let mut titles = Vec::new();
        let mut bookmark = bookmarks.iter().next();
        while let Some(node) = bookmark {
            if destination_page(&node) == page
                && let Some(title) = node.title()
            {
                titles.push(title);
            }
            collect_child_titles(&node, page, &mut titles);
            bookmark = node.next_sibling();
        }
        titles
    }

    /// Char indexes where paragraphs end on a page, computed from the
    /// text layer's character boxes: characters are grouped into visual
    /// lines, and line ends whose vertical gap to the next line clearly
    /// exceeds the page's median line pitch are paragraph boundaries.
    /// `char_count` must match the page's text length (the frontend and
    /// engine must agree on the text); returns None when they don't or the
    /// page can't be read — callers fall back to sentence-only chunking.
    pub fn paragraph_breaks(
        &self,
        doc_id: u32,
        page_index: PdfPageIndex,
        char_count: usize,
    ) -> Option<Vec<usize>> {
        if char_count == 0 {
            return Some(Vec::new());
        }
        let docs = self.docs.lock().unwrap();
        let doc = docs.get(&doc_id)?;
        let pages = doc.pages();
        let page = if page_index >= 0 && page_index < pages.len() {
            pages.get(page_index).ok()?
        } else {
            return None;
        };
        let text = page.text().ok()?;
        if text.len() as usize != char_count {
            return None;
        }

        // Char index -> top of its box (PDF points, y grows upward);
        // chars without a box inherit the last seen top.
        let mut indexed_tops: Vec<(usize, f32)> = Vec::with_capacity(char_count);
        for (index, ch) in (0..).zip(text.chars().iter()) {
            let top = ch
                .loose_bounds()
                .ok()
                .map(|rect| rect.top().value)
                .or_else(|| indexed_tops.last().map(|(_, top)| *top));
            if let Some(top) = top {
                indexed_tops.push((index, top));
            }
        }
        if indexed_tops.is_empty() {
            return Some(Vec::new());
        }

        // Group into lines: a new line starts when the top moves by more
        // than a small fraction of the page height.
        let mut lines: Vec<(usize, f32)> = Vec::new();
        let mut line_start = indexed_tops[0];
        for &(index, top) in &indexed_tops {
            if (line_start.1 - top).abs() > 1.5 {
                lines.push(line_start);
                line_start = (index, top);
            }
        }
        lines.push(line_start);
        if lines.len() < 3 {
            return Some(Vec::new());
        }

        let mut pitches: Vec<f32> = lines.windows(2).map(|w| (w[0].1 - w[1].1).abs()).collect();
        pitches.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = pitches[pitches.len() / 2];

        let mut breaks = Vec::new();
        for window in lines.windows(2) {
            let gap = (window[0].1 - window[1].1).abs();
            if gap > median * 1.6 {
                let next_start = window[1].0;
                if next_start > 0 {
                    breaks.push(next_start - 1);
                }
            }
        }
        Some(breaks)
    }

    /// The document's outline (table of contents) as a page-indexed tree.
    pub fn outline(&self, doc_id: u32) -> Result<Vec<OutlineNode>, ApiError> {
        let docs = self.docs.lock().unwrap();
        let doc = docs.get(&doc_id).ok_or(ApiError::UnknownDocument)?;
        let bookmarks = doc.bookmarks();

        let mut roots = Vec::new();
        let mut bookmark = bookmarks.iter().next();
        while let Some(node) = bookmark {
            roots.push(outline_node(&node, 0));
            bookmark = node.next_sibling();
        }
        Ok(roots)
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
            bitmap.as_image().map_err(|e| ApiError::RenderFailed {
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
    max_bytes: usize,
}

type CacheKey = (u32, PdfPageIndex, u32);

/// Soft cap on cached rendered PNGs (~192 MB).
const RENDER_CACHE_MAX_BYTES: usize = 192 * 1024 * 1024;

impl RenderCache {
    fn new() -> Self {
        Self::with_max_bytes(RENDER_CACHE_MAX_BYTES)
    }

    fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
            tick: 0,
            max_bytes,
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
        while self.total_bytes > self.max_bytes && self.entries.len() > 1 {
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

fn destination_page(bookmark: &PdfBookmark<'_>) -> PdfPageIndex {
    bookmark
        .destination()
        .and_then(|dest| dest.page_index().ok())
        .unwrap_or(-1)
}

fn collect_child_titles(bookmark: &PdfBookmark<'_>, page: PdfPageIndex, titles: &mut Vec<String>) {
    let mut child = bookmark.first_child();
    while let Some(node) = child {
        if destination_page(&node) == page
            && let Some(title) = node.title()
        {
            titles.push(title);
        }
        collect_child_titles(&node, page, titles);
        child = node.next_sibling();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(bytes: u8) -> Arc<Vec<u8>> {
        Arc::new(vec![0u8; bytes as usize])
    }

    #[test]
    fn cache_hit_and_miss() {
        let mut cache = RenderCache::with_max_bytes(1000);
        assert!(cache.get(&(1, 0, 800)).is_none());
        cache.insert((1, 0, 800), entry(10));
        assert_eq!(cache.get(&(1, 0, 800)).unwrap().len(), 10);
        assert!(cache.get(&(1, 1, 800)).is_none());
    }

    #[test]
    fn eviction_removes_least_recently_used() {
        let mut cache = RenderCache::with_max_bytes(35);
        cache.insert((1, 0, 800), entry(10)); // LRU order: p0
        cache.insert((1, 1, 800), entry(10)); // p0, p1
        cache.insert((1, 2, 800), entry(10)); // p0, p1, p2
        assert_eq!(cache.total_bytes, 30);
        cache.get(&(1, 0, 800)); // touch p0 -> p1, p2, p0

        cache.insert((1, 3, 800), entry(10)); // 40 > 35 -> evicts p1

        assert!(cache.get(&(1, 1, 800)).is_none());
        assert!(cache.get(&(1, 0, 800)).is_some());
        assert!(cache.get(&(1, 2, 800)).is_some());
        assert!(cache.get(&(1, 3, 800)).is_some());
        assert_eq!(cache.total_bytes, 30);
    }

    #[test]
    fn reinserting_same_key_keeps_total_bytes_consistent() {
        let mut cache = RenderCache::with_max_bytes(30);
        cache.insert((1, 0, 800), entry(10));
        cache.insert((1, 0, 800), entry(20));
        cache.insert((1, 0, 800), entry(10));
        assert_eq!(cache.total_bytes, 10);
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn retain_doc_drops_only_that_document() {
        let mut cache = RenderCache::with_max_bytes(1000);
        cache.insert((1, 0, 800), entry(10));
        cache.insert((2, 0, 800), entry(10));
        cache.retain_doc(1);
        assert!(cache.get(&(1, 0, 800)).is_none());
        assert!(cache.get(&(2, 0, 800)).is_some());
        assert_eq!(cache.total_bytes, 10);
    }

    fn s(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn find_matches_reports_all_occurrences() {
        // "the" appears at 0, 4 ("theme") and 13 ("the book")
        let haystack = s("the theme of the book");
        let matches = find_matches(&haystack, &s("the"), true);
        assert_eq!(matches, vec![(0, 3), (4, 7), (13, 16)]);
    }

    #[test]
    fn find_matches_case_insensitive_by_default() {
        let matches = find_matches(&s("Alpha beta ALPHA"), &s("alpha"), false);
        assert_eq!(matches, vec![(0, 5), (11, 16)]);
        assert!(find_matches(&s("Alpha"), &s("alpha"), true).is_empty());
    }

    #[test]
    fn normalization_collapses_line_breaks_to_spaces() {
        let raw = s("end of line.\r\nNew chapter");
        let (normalized, map) = normalize_for_search(&raw);
        let text: String = normalized.iter().collect();
        assert_eq!(text, "end of line. New chapter");

        // A cross-line query maps back to the full \r\n span.
        let matches = find_matches(&normalized, &s("line. new"), false);
        assert_eq!(matches.len(), 1);
        let (norm_start, norm_end) = matches[0];
        let start = map[norm_start];
        let end = if norm_end < map.len() {
            map[norm_end]
        } else {
            raw.len()
        };
        let matched: String = raw[start..end].iter().collect();
        assert_eq!(matched, "line.\r\nNew");
    }

    #[test]
    fn find_matches_handles_empty_needle_and_overflow() {
        assert!(find_matches(&s("abc"), &[], false).is_empty());
        assert!(find_matches(&s("ab"), &s("abc"), false).is_empty());
    }
}
