// Rendering smoke test: builds a blank A4 document in memory and renders it
// to PNG through the same configuration the app uses.
// Requires the PDFium library — run `npm run fetch:pdfium` before `cargo test`.

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
