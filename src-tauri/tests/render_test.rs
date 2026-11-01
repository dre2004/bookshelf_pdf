// Rendering smoke test: builds a blank A4 document in memory and renders it
// to PNG through the same configuration the app uses.
// Requires the PDFium library — run `npm run fetch:pdfium` before `cargo test`.

use bookshelf_pdf_lib::pdfium_engine::collect_page_links;
use pdfium_render::prelude::*;
use std::{io::Cursor, path::PathBuf};

fn bind_pdfium() -> Pdfium {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut candidates = vec![
        manifest_dir.join("resources/pdfium/pdfium.dll"),
        manifest_dir.join("resources/pdfium/libpdfium.dylib"),
        manifest_dir.join("resources/pdfium/libpdfium.so"),
    ];
    if let Ok(path) = std::env::var("BOOKSHELF_PDFIUM_PATH") {
        candidates.insert(0, PathBuf::from(path));
    }
    let path = candidates
        .iter()
        .find(|p| p.is_file())
        .expect("PDFium library not found; run `npm run fetch:pdfium` first");
    let bindings = Pdfium::bind_to_library(path).unwrap();
    Pdfium::new(bindings)
}

#[test]
fn render_blank_page_to_png() {
    let pdfium = bind_pdfium();
    let mut doc = pdfium.create_new_pdf().unwrap();
    doc.pages_mut()
        .create_page_at_end(PdfPagePaperSize::a4())
        .unwrap();

    let page = doc.pages().get(0).unwrap();
    let config = PdfRenderConfig::new()
        .set_target_width(400)
        .set_format(PdfBitmapFormat::BGRA)
        .set_clear_color(PdfColor::WHITE);
    let bitmap = page.render_with_config(&config).unwrap();
    assert!(
        (380..=420).contains(&bitmap.width()),
        "unexpected width {}",
        bitmap.width()
    );

    let image = bitmap.as_image().unwrap();
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    assert_eq!(&png[0..4], b"\x89PNG", "output is not a PNG");
    assert!(
        png.len() > 500,
        "PNG suspiciously small: {} bytes",
        png.len()
    );
}

#[test]
fn page_links_extracts_uri_link_geometry() {
    let pdfium = bind_pdfium();
    let mut doc = pdfium.create_new_pdf().unwrap();
    doc.pages_mut()
        .create_page_at_end(PdfPagePaperSize::a4())
        .unwrap();

    let mut page = doc.pages_mut().get(0).unwrap();
    let mut annotation = page
        .annotations_mut()
        .create_link_annotation("https://example.com/docs")
        .unwrap();
    // PdfRect::new_from_values takes (bottom, left, top, right): a box from
    // (100, 700) to (300, 720) in PDF points.
    annotation
        .set_bounds(PdfRect::new_from_values(700.0, 100.0, 720.0, 300.0))
        .unwrap();

    // A fresh in-memory page other than the link's target: re-fetch the
    // page and read links back through the engine's collector.
    let links = collect_page_links(&doc.pages().get(0).unwrap());
    assert_eq!(links.len(), 1, "expected exactly one link: {links:?}");
    let link = &links[0];
    assert_eq!(
        link.uri.as_deref(),
        Some("https://example.com/docs"),
        "uri wrong: {:?}",
        link.uri
    );
    assert_eq!(link.dest_page, None);
    let [x, y, w, h] = link.rect;
    assert!((x - 100.0).abs() < 0.5, "x was {x}");
    assert!((y - 700.0).abs() < 0.5, "y was {y}");
    assert!((w - 200.0).abs() < 0.5, "width was {w}");
    assert!((h - 20.0).abs() < 0.5, "height was {h}");
}

#[test]
fn page_links_empty_when_page_has_none() {
    let pdfium = bind_pdfium();
    let mut doc = pdfium.create_new_pdf().unwrap();
    doc.pages_mut()
        .create_page_at_end(PdfPagePaperSize::a4())
        .unwrap();
    let links = collect_page_links(&doc.pages().get(0).unwrap());
    assert!(links.is_empty(), "unexpected links: {links:?}");
}

/// Dev probe against a real book with internal links. Skipped unless
/// BOOKSHELF_LINKS_PDF points at a PDF:
///   BOOKSHELF_LINKS_PDF="Some Book.pdf" cargo test --test render_test real_pdf
#[test]
fn real_pdf_link_scan() {
    let Ok(path) = std::env::var("BOOKSHELF_LINKS_PDF") else {
        eprintln!("skipping: BOOKSHELF_LINKS_PDF not set");
        return;
    };
    let pdfium = bind_pdfium();
    let doc = pdfium
        .load_pdf_from_file(&path, None)
        .unwrap_or_else(|e| panic!("opening {path}: {e}"));
    let mut internal = 0;
    let mut external = 0;
    for page in doc.pages().iter() {
        for link in collect_page_links(&page) {
            if link.dest_page.is_some() {
                internal += 1;
            }
            if link.uri.is_some() {
                external += 1;
            }
        }
    }
    eprintln!(
        "{} pages: {internal} internal links, {external} external links",
        doc.pages().len()
    );
    assert!(internal + external > 0, "no links found in {path}");
}
