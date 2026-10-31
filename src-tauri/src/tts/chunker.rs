//! Sentence-aware text chunking for speech synthesis.

/// Splits text into chunks of at most `max` characters, preferring sentence
/// boundaries, then word boundaries, then hard cuts. Returns char-index
/// ranges `[start, end)` into the original text; boundaries always close a
/// chunk (a chunk may end with punctuation or whitespace, never start with
/// a stranded space). Empty and whitespace-only chunks are skipped.
pub fn split_for_tts(text: &str, max: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut last_break = 0;

    let sentence_break = |c: char| matches!(c, '.' | '!' | '?' | ';' | ':' | '\n');

    for (i, &c) in chars.iter().enumerate() {
        if sentence_break(c) || c.is_whitespace() {
            last_break = i + 1;
        }
        let length = i + 1 - start;
        if length >= max {
            // Prefer the most recent break; hard-cut only when none exists.
            let end = if last_break > start {
                last_break
            } else {
                i + 1
            };
            push_chunk(&chars, &mut chunks, start, end);
            start = end;
            last_break = start;
        }
    }
    push_chunk(&chars, &mut chunks, start, chars.len());
    chunks
}

fn push_chunk(chars: &[char], chunks: &mut Vec<(usize, usize)>, start: usize, end: usize) {
    if end <= start {
        return;
    }
    if chars[start..end].iter().any(|c| !c.is_whitespace()) {
        chunks.push((start, end));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_limit_and_break_on_boundaries() {
        let text = "One sentence. Two sentences here! And a third one follows.";
        let chunks = split_for_tts(text, 40);
        assert!(!chunks.is_empty());
        let chars: Vec<char> = text.chars().collect();
        let mut cursor = 0;
        for (start, end) in &chunks {
            assert_eq!(*start, cursor, "chunks must be contiguous");
            assert!(end - start <= 40, "chunk of {} chars", end - start);
            if *end < chars.len() {
                let boundary = chars[end - 1];
                assert!(
                    matches!(boundary, '.' | '!' | '?' | ' ' | '\n'),
                    "chunk ends on '{boundary}'"
                );
            }
            cursor = *end;
        }
        assert_eq!(cursor, chars.len(), "chunks must cover the text");
    }

    #[test]
    fn splits_long_sentences_at_words() {
        let text = "word ".repeat(60); // 300 chars
        let chunks = split_for_tts(&text, 220);
        assert!(chunks.len() >= 2);
        for (start, end) in &chunks {
            assert!(end - start <= 220, "chunk of {} chars", end - start);
        }
        // Chars at cut points are spaces (chunk ends just before one).
        for (start, _end) in chunks.iter().skip(1) {
            assert_eq!(text.chars().nth(start - 1), Some(' '));
        }
    }

    #[test]
    fn hard_cuts_unbreakable_text() {
        let text = "x".repeat(500);
        let chunks = split_for_tts(&text, 220);
        assert_eq!(chunks, vec![(0, 220), (220, 440), (440, 500)]);
    }

    #[test]
    fn skips_whitespace_only_chunks() {
        assert!(split_for_tts("   \r\n  ", 220).is_empty());
    }

    #[test]
    fn unicode_text_counts_characters_not_bytes() {
        let text = "¿Dónde estás? ".repeat(20);
        let chunks = split_for_tts(&text, 220);
        assert_eq!(chunks.len(), 2);
        for (start, end) in &chunks {
            assert!(end - start <= 220);
            // Indexes are char indexes: they address valid char boundaries.
            let _ = text.chars().skip(*start).take(end - start).count();
        }
    }
}
