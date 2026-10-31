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
/// A sentence ends at `.`, `!` or `?` — never at a newline (PDF text
/// extraction emits newlines at every visual line break, which are layout,
/// not meaning). Exceptions where `.` does not end a sentence: decimals
/// (`3.14`), section numbering (`0.1`, `3.2.1`), a lowercase word
/// following (`e.g. apples`), and common abbreviations (`Mr.`, `etc.`).
///
/// Consecutive sentences group into a selection of up to
/// [`MAX_SENTENCES`] or [`MAX_SELECTION_CHARS`], whichever comes first.
/// Paragraph boundaries (char indexes computed from the page's text
/// geometry) always end a selection. Ranges are `[start, end)` into the
/// original text.
pub fn split_for_narration(text: &str, paragraph_breaks: &[usize]) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let sentences = split_sentences(&chars);

    // Headings and list items usually carry no sentence terminator, so a
    // heading rides glued to the first body sentence. Paragraph breaks
    // (from page geometry) split any sentence that spans them, giving
    // headings their own chunk.
    let mut split_sentences_out: Vec<(usize, usize)> = Vec::with_capacity(sentences.len());
    for (start, end) in sentences {
        let mut piece_start = start;
        for &brk in paragraph_breaks {
            if brk > piece_start && brk + 1 < end {
                push_chunk(&chars, &mut split_sentences_out, piece_start, brk + 1);
                piece_start = brk + 1;
            }
        }
        push_chunk(&chars, &mut split_sentences_out, piece_start, end);
    }

    let sentences = split_sentences_out;
    let mut chunks = Vec::new();

    let mut index = 0;
    while index < sentences.len() {
        let mut group_start = sentences[index].0;
        let mut group_end = sentences[index].1;
        let mut used = 0;
        while index < sentences.len() {
            let (start, end) = sentences[index];
            // Capacity check first: close the group before this sentence.
            let would_exceed =
                used > 0 && (used + 1 > MAX_SENTENCES || end - group_start > MAX_SELECTION_CHARS);
            if used > 0 && would_exceed {
                break;
            }
            if used == 0 {
                group_start = start;
            }
            // A paragraph boundary at this sentence's end closes the group
            // AFTER including it — the breaking sentence belongs to its
            // own paragraph.
            let at_break = paragraph_breaks.contains(&end.saturating_sub(1))
                || paragraph_breaks.contains(&end);
            group_end = end;
            used += 1;
            index += 1;
            if at_break {
                break;
            }
        }
        if group_end - group_start > MAX_SELECTION_CHARS {
            for (word_start, word_end) in
                split_at_words(&chars, group_start, group_end, MAX_SELECTION_CHARS)
            {
                push_chunk(&chars, &mut chunks, word_start, word_end);
            }
        } else {
            push_chunk(&chars, &mut chunks, group_start, group_end);
        }
    }
    chunks
}

pub const MAX_SENTENCES: usize = 3;
pub const MAX_SELECTION_CHARS: usize = 400;

const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "no", "vol", "chap", "pp", "fig", "e.g",
    "i.e", "etc", "vs", "cf", "al", "ca", "ed", "eds",
];

/// Sentence spans under the rules above. Trailing terminator and spaces up
/// to the next word are included in each span.
fn split_sentences(chars: &[char]) -> Vec<(usize, usize)> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let terminates = match chars[i] {
            '!' | '?' => true,
            '.' => period_ends_sentence(chars, i),
            _ => false,
        };
        if !terminates {
            continue;
        }
        let mut end = i + 1;
        while end < chars.len() && (chars[end] == ' ' || chars[end] == '\t') {
            end += 1;
        }
        if end > start {
            push_chunk(chars, &mut sentences, start, end);
        }
        start = end;
    }
    if start < chars.len() {
        push_chunk(chars, &mut sentences, start, chars.len());
    }
    sentences
}

/// Whether the period at `i` ends a sentence.
fn period_ends_sentence(chars: &[char], i: usize) -> bool {
    // Decimal / section numbering: period glued to a digit on the right.
    if matches!(chars.get(i + 1), Some(next) if next.is_ascii_digit()) {
        return false;
    }
    // Abbreviation: the word ending at this period is a known short form.
    let word_before: String = chars[..i]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .next_back()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if ABBREVIATIONS.contains(&word_before.as_str()) {
        return false;
    }
    // Lowercase follower: "…, e.g. apples" continues the sentence.
    let mut j = i + 1;
    while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
        j += 1;
    }
    if matches!(chars.get(j), Some(follower) if follower.is_lowercase()) {
        return false;
    }
    true
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

    fn cover(text: &str, chunks: &[(usize, usize)]) -> String {
        chunks
            .iter()
            .map(|(s, e)| text.chars().skip(*s).take(e - s).collect::<String>())
            .collect()
    }

    fn chunk_text(text: &str, chunks: &[(usize, usize)], index: usize) -> String {
        let (s, e) = chunks[index];
        text.chars().skip(s).take(e - s).collect()
    }

    #[test]
    fn groups_up_to_three_sentences() {
        let text = "One sentence. Two sentence. Three sentence. Four sentence.";
        let chunks = split_for_narration(text, &[]);
        assert_eq!(chunks.len(), 2, "expected 3+1 grouping: {chunks:?}");
        assert_eq!(cover(text, &chunks).replace(' ', ""), text.replace(' ', ""));
    }

    #[test]
    fn line_breaks_do_not_split_sentences() {
        let text = "This sentence wraps across\r\ntwo lines on the page. Next one.";
        let chunks = split_for_narration(text, &[]);
        let first = chunk_text(text, &chunks, 0);
        assert!(
            first.contains("wraps across"),
            "split at line break: {first:?}"
        );
        assert!(first.contains("page."), "split at line break: {first:?}");
    }

    #[test]
    fn decimals_and_numbering_are_not_terminators() {
        let text = "Pi is 3.14 exactly. 0.1 How to read this book.";
        let chunks = split_for_narration(text, &[]);
        let joined = cover(text, &chunks);
        assert!(joined.contains("3.14"), "decimal split: {joined:?}");
        let title = chunks
            .iter()
            .map(|(s, e)| text.chars().skip(*s).take(e - s).collect::<String>())
            .find(|c| c.contains("How to read"))
            .expect("title chunk missing");
        assert!(title.contains("0.1"), "number split off: {title:?}");
    }

    #[test]
    fn abbreviations_and_lowercase_followers_continue() {
        let text = "See Fig. 3 for details, e.g. the tables. Then Mr. Smith left.";
        let chunks = split_for_narration(text, &[]);
        for (s, e) in &chunks {
            let chunk: String = text.chars().skip(*s).take(e - s).collect();
            let trimmed = chunk.trim_start();
            assert!(
                !trimmed.starts_with(|c: char| c.is_lowercase()),
                "chunk starts mid-sentence: {chunk:?}"
            );
            assert!(!trimmed.starts_with("Smith"), "abbrev split: {chunk:?}");
        }
    }

    #[test]
    fn paragraph_breaks_always_close_a_selection() {
        let text = "First short one. Second short one. Third here. New paragraph text.";
        // Char index just past the period of "Second short one.".
        let chars: Vec<char> = text.chars().collect();
        let second_end = chars.iter().position(|&c| c == '.').unwrap() + 1;
        let break_at = chars
            .iter()
            .enumerate()
            .find(|(i, c)| **c == '.' && *i > second_end)
            .map(|(i, _)| i + 1)
            .unwrap();
        let chunks = split_for_narration(text, &[break_at]);
        assert!(chunks.len() >= 2, "paragraph break ignored: {chunks:?}");
        let later = chunk_text(text, &chunks, 1);
        assert!(
            later.trim_start().starts_with("Third"),
            "paragraph break ignored: {later:?}"
        );
    }

    #[test]
    fn paragraph_break_splits_heading_without_period_from_body() {
        // Real-world shape: heading has no terminator; geometry says the
        // paragraph ends after "book" (char index of the newline).
        let text = "0.1 How to read this book\nYou will read this book. It helps.";
        let chars: Vec<char> = text.chars().collect();
        let brk = chars.iter().position(|&c| c == '\n').unwrap();
        let chunks = split_for_narration(text, &[brk]);
        let first: String = chars[chunks[0].0..chunks[0].1].iter().collect();
        assert!(
            first.contains("How to read this book") && !first.contains("You will"),
            "heading not separated: {first:?}"
        );
    }

    #[test]
    fn overlong_sentence_splits_at_words() {
        let text = "word ".repeat(120); // 600 chars, no punctuation
        let chunks = split_for_narration(&text, &[]);
        assert!(chunks.len() >= 2);
        for (start, end) in &chunks {
            assert!(end - start <= 401, "chunk of {} chars", end - start);
        }
    }
}
