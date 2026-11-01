import io

# ---- commands.rs: fetch code blocks from the page text, pass handling ----
p = "src/commands.rs"
s = io.open(p, encoding="utf-8").read()
old = """    let (breaks, outline_titles) = doc_id
        .map(|id| {
            (
                pdfium
                    .paragraph_breaks(id, page, text.chars().count())
                    .unwrap_or_default(),
                pdfium.outline_titles_for_page(id, page),
            )
        })
        .unwrap_or_default();
    engine.speak(&text, page, &voice_id, rate as f32, breaks, outline_titles)"""
new = """    let (breaks, outline_titles, code_blocks) = doc_id
        .map(|id| {
            (
                pdfium
                    .paragraph_breaks(id, page, text.chars().count())
                    .unwrap_or_default(),
                pdfium.outline_titles_for_page(id, page),
                pdfium
                    .page_text(id, page)
                    .map(|data| {
                        // Char offsets in data.chars map 1:1 to the text
                        // the frontend extracted (same source, same order).
                        data.code_blocks
                            .into_iter()
                            .map(|(s, e)| (s, e))
                            .collect::<Vec<(usize, usize)>>()
                    })
                    .unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    engine.speak(
        &text,
        page,
        &voice_id,
        rate as f32,
        breaks,
        outline_titles,
        code_blocks,
        code_handling.unwrap_or_else(|| "skip".to_string()),
    )"""
assert old in s, "tts_speak body anchor missing"
s = s.replace(old, new)
s = s.replace(
"    rate: f64,\n    doc_id: Option<u32>,\n) -> Result<(), String> {",
"    rate: f64,\n    doc_id: Option<u32>,\n    code_handling: Option<String>,\n) -> Result<(), String> {")
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("commands patched")

# ---- mod.rs: classify_code_chunks helper ----
p = "src/tts/mod.rs"
s = io.open(p, encoding="utf-8").read()
helper = r'''
/// Marks each narration chunk as code when ≥60% of its characters fall
/// inside a detected code block. Consecutive code chunks are collapsed so
/// a multi-chunk listing yields one announcement, not several.
fn classify_code_chunks(
    chunks: &[(usize, usize)],
    code_blocks: &[(usize, usize)],
) -> Vec<bool> {
    chunks
        .iter()
        .map(|(start, end)| {
            let length = end.saturating_sub(*start).max(1);
            let covered: usize = code_blocks
                .iter()
                .map(|(bs, be)| {
                    let lo = (*start).max(*bs);
                    let hi = (*end).min(*be);
                    hi.saturating_sub(lo)
                })
                .sum();
            covered * 10 >= length * 6
        })
        .collect()
}
'''
anchor = "fn gap_samples("
assert anchor in s and "classify_code_chunks" in s
s = s.replace(anchor, helper + "\n" + anchor, 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("classify helper added")
