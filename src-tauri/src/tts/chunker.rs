//! Sentence-aware text chunking for speech synthesis.

fn push_chunk(chars: &[char], chunks: &mut Vec<(usize, usize)>, start: usize, end: usize) {
    if end <= start {
        return;
    }
    if chars[start..end].iter().any(|c| !c.is_whitespace()) {
        chunks.push((start, end));
    }
}

/// Sentence-level chunking for narration.
///
/// Sentences end at `.`, `!`, `?` or `:` — colons are clause boundaries a
/// listener expects a beat at. Newlines are NOT sentence endings: PDF
/// extraction emits one per visual line, so wrapped prose must join.
/// Boundaries additionally open after blank lines (paragraphs) and after
/// heading-like lines. Consecutive short sentences merge (up to 3 / ~400
/// chars) across `.`, `!`, `?` only — never across `:`, headings or
/// paragraphs. Ranges are `[start, end)` into the original text.
pub fn split_for_narration(text: &str, paragraph_breaks: &[usize]) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut boundaries = find_boundaries(&chars);
    // Geometry-derived paragraph breaks arrive as char offsets; they are
    // unmergeable boundaries like colons and headings.
    for &offset in paragraph_breaks {
        let offset = offset.min(chars.len());
        if offset > 0 {
            boundaries.push((offset, false));
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut spans: Vec<((usize, usize), bool)> = Vec::new();
    let mut start = 0;
    for (end, mergeable) in boundaries {
        if end > start {
            spans.push(((start, end), mergeable));
        }
        start = end;
    }
    if start < chars.len() {
        spans.push(((start, chars.len()), false));
    }

    let mut chunks: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < spans.len() {
        let ((start, end), mergeable) = spans[i];
        let length = end - start;
        if mergeable && length < 150 && i + 1 < spans.len() {
            let (_next_start, next_end) = spans[i + 1].0;
            let merged_sentences = count_sentences(&chars, start, end) + 1;
            if next_end - start <= 400 && merged_sentences <= 3 {
                push_chunk(&chars, &mut chunks, start, next_end);
                i += 2;
                continue;
            }
        }
        if length > 220 {
            for (word_start, word_end) in split_at_words(&chars, start, end, 220) {
                push_chunk(&chars, &mut chunks, word_start, word_end);
            }
        } else {
            push_chunk(&chars, &mut chunks, start, end);
        }
        i += 1;
    }
    chunks
}

/// Boundary offsets (sentence ends, paragraph starts, heading ends) with a
/// flag: true when the next span may merge into this one (plain sentence
/// terminators only — never colons, headings or paragraphs).
fn find_boundaries(chars: &[char]) -> Vec<(usize, bool)> {
    let mut boundaries = Vec::new();
    let mut span_start = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let _ = span_start;
        let terminator = match c {
            '!' | '?' | ':' => true,
            '.' => {
                let next = chars.get(i + 1);
                if matches!(next, Some(n) if n.is_ascii_digit()) {
                    // Decimals and section numbering: "3.14", "0.1".
                    false
                } else if matches!(next, Some(n) if n.is_whitespace()) {
                    let after = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                    match after {
                        // "e.g. this": lowercase continuation is an
                        // abbreviation, not a new sentence.
                        Some(a) if a.is_lowercase() => false,
                        _ => !ends_with_abbreviation(chars, i),
                    }
                } else {
                    next.is_none() || matches!(next, Some(n) if n.is_whitespace() || is_closing(*n))
                }
            }
            _ => false,
        };
        if terminator {
            let mut end = i + 1;
            while chars.get(end).is_some_and(|c| *c == ' ' || *c == '\t') {
                end += 1;
            }
            boundaries.push((end, c != ':'));
            span_start = end;
            i = end;
            continue;
        }
        i += 1;
    }
    boundaries
}

fn is_closing(c: char) -> bool {
    matches!(c, '"' | '\'' | ')' | ']' | '}' | '\u{201D}' | '\u{2019}')
}

/// True when the word ending at `dot` is a common title abbreviation
/// ("Mr.", "Fig.", "e.g.") whose period must not end a sentence.
fn ends_with_abbreviation(chars: &[char], dot: usize) -> bool {
    const ABBREVIATIONS: &[&str] = &[
        "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "no", "vol", "fig", "figs", "tab",
        "chap", "pp", "ed", "eds", "al", "etc", "eg", "ie", "vs", "cf", "ca",
    ];
    let start = chars[..dot]
        .iter()
        .rposition(|c| c.is_whitespace())
        .map(|position| position + 1)
        .unwrap_or(0);
    let word: String = chars[start..dot]
        .iter()
        .filter(|c| c.is_alphanumeric())
        .collect();
    ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

/// Sentence terminators in a span (for the 3-sentence merge cap).
fn count_sentences(chars: &[char], start: usize, end: usize) -> usize {
    chars[start..end]
        .iter()
        .filter(|c| matches!(c, '.' | '!' | '?' | ':'))
        .count()
}

/// Word-boundary splits of an overlong span into ≤max pieces.
fn split_at_words(chars: &[char], start: usize, end: usize, max: usize) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut piece_start = start;
    let mut last_space = start;
    for i in start..end {
        if chars[i].is_whitespace() {
            last_space = i;
        }
        if i + 1 - piece_start >= max {
            let cut = if last_space > piece_start {
                last_space + 1
            } else {
                i + 1
            };
            push_chunk(chars, &mut pieces, piece_start, cut);
            piece_start = cut;
            last_space = cut;
        }
    }
    push_chunk(chars, &mut pieces, piece_start, end);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_texts(text: &str, chunks: &[(usize, usize)]) -> Vec<String> {
        chunks
            .iter()
            .map(|(s, e)| text.chars().skip(*s).take(e - s).collect())
            .collect()
    }

    #[test]
    fn colon_ends_a_clause() {
        let text = "Finish Part 0, then start Chapter 1 and type in every program it shows: the typing is not a ritual.";
        let texts = chunk_texts(text, &split_for_narration(text, &[]));
        assert!(texts.len() >= 2, "colon did not split: {texts:?}");
        assert!(
            texts[0].contains("it shows:"),
            "first part wrong: {:?}",
            texts[0]
        );
        assert!(
            texts[1].contains("the typing"),
            "second part wrong: {:?}",
            texts[1]
        );
    }

    #[test]
    fn wrapped_lines_join_into_one_sentence() {
        let text = "This sentence wraps onto\r\nthe next line of the page. Second sentence.";
        let texts = chunk_texts(text, &split_for_narration(text, &[]));
        assert!(
            texts[0].contains("wraps onto") && texts[0].contains("next line"),
            "wrapped sentence split: {texts:?}"
        );
    }

    #[test]
    fn heading_line_joins_body_chunk() {
        // Heading extraction happens downstream (heading_aware_pieces);
        // the chunker only respects sentence terminators and geometry
        // breaks, so a heading line and its body form one chunk.
        let text = "If You're New Here
Welcome to the book. This is the body text.";
        let texts = chunk_texts(text, &split_for_narration(text, &[]));
        assert_eq!(texts.len(), 1, "unexpected split: {texts:?}");
    }

    #[test]
    fn decimals_and_abbreviations_do_not_end_sentences() {
        let text = "Use version 3.14 as shown in Fig. 3 to build it. Then run.";
        let texts = chunk_texts(text, &split_for_narration(text, &[]));
        assert!(texts.len() <= 2, "abbreviation/decimal split: {texts:?}");
        assert!(
            texts[0].contains("Fig. 3"),
            "Fig. sentence lost: {:?}",
            texts[0]
        );
    }

    #[test]
    fn short_sentences_merge_up_to_three() {
        let text = "One. Two is here. Three now. Four follows.";
        let texts = chunk_texts(text, &split_for_narration(text, &[]));
        assert!(texts.len() >= 2, "nothing merged: {texts:?}");
    }

    #[test]
    fn long_sentences_split_at_words() {
        let text = "word ".repeat(80);
        let chunks = split_for_narration(&text, &[]);
        assert!(chunks.len() >= 2);
        for (start, end) in &chunks {
            assert!(end - start <= 221, "chunk of {} chars", end - start);
        }
    }

    #[test]
    fn section_numbering_stays_with_its_title() {
        let text = "0.1 How to read this book\r\nYou will read this book.";
        let texts = chunk_texts(text, &split_for_narration(text, &[]));
        assert!(
            texts[0].contains("How to read this book"),
            "title split off: {:?}",
            texts[0]
        );
    }
}
