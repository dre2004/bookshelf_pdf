//! Sentence-aware text chunking for speech synthesis.

fn push_chunk(chars: &[char], chunks: &mut Vec<(usize, usize)>, start: usize, end: usize) {
    if end <= start {
        return;
    }
    if chars[start..end].iter().any(|c| !c.is_whitespace()) {
        chunks.push((start, end));
    }
}

/// Sentence-level chunking for narration: one sentence per chunk, or two
/// merged when both are short. Overlong sentences split at word
/// boundaries. Section numbering ("0.1", "3.2.1") is not treated as
/// sentence punctuation. Ranges are `[start, end)` into the original text.
pub fn split_for_narration(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let sentences = split_sentences(&chars);
    let mut chunks = Vec::new();

    let mut index = 0;
    while index < sentences.len() {
        let (start, end) = sentences[index];
        let length = end - start;
        // Merge a short sentence with the next one when both are short —
        // but never across a paragraph boundary (newline), so headings and
        // paragraph starts stand as their own chunk.
        let ends_paragraph = chars[end - 1] == '\n';
        if length < 100 && !ends_paragraph && index + 1 < sentences.len() {
            let (_next_start, next_end) = sentences[index + 1];
            if next_end - start < 200 {
                push_chunk(&chars, &mut chunks, start, next_end);
                index += 2;
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
        index += 1;
    }
    chunks
}

/// Sentence spans, where a terminator is `.`, `!`, `?`, `:` or a newline —
/// except a period directly followed by a digit (section numbers,
/// decimals).
fn split_sentences(chars: &[char]) -> Vec<(usize, usize)> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for i in 0..chars.len() {
        let c = chars[i];
        let terminator = matches!(c, '!' | '?' | ':' | '\n')
            || (c == '.' && chars.get(i + 1) != Some(&')') && {
                let next = chars.get(i + 1);
                // "0.1" / "3.14": period glued to a digit is not a
                // terminator; ". 1" after a space can be a list marker —
                // treat as terminator only when followed by whitespace and
                // an uppercase/digit-start word handled by caller chunks.
                match next {
                    Some(next) if next.is_ascii_digit() && c == '.' => false,
                    _ => c == '.',
                }
            });
        if !terminator {
            continue;
        }
        // Include trailing terminator (and spaces up to the next word).
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

    #[test]
    fn narration_splits_per_sentence() {
        let text = "First sentence here. Second one follows! Third?";
        let chunks = split_for_narration(text);
        // All short sentences: merged pairwise -> 2 chunks.
        assert_eq!(chunks.len(), 2);
        for (start, end) in &chunks {
            assert!(end - start <= 200, "chunk too long: {}", end - start);
        }
        let covered: String = chunks
            .iter()
            .map(|(s, e)| text.chars().skip(*s).take(e - s).collect::<String>())
            .collect();
        assert_eq!(covered.replace(' ', ""), text.replace(' ', ""));
    }

    #[test]
    fn narration_keeps_section_numbering_with_its_title() {
        let text = "0.1 How to read this book. This is the first sentence.";
        let chunks = split_for_narration(text);
        let first: String = text
            .chars()
            .skip(chunks[0].0)
            .take(chunks[0].1 - chunks[0].0)
            .collect();
        assert!(
            first.contains("How to read this book"),
            "title split off: {first:?}"
        );
    }

    #[test]
    fn narration_splits_long_sentences_at_words() {
        let text = "word ".repeat(80); // 400 chars, no punctuation
        let chunks = split_for_narration(&text);
        assert!(chunks.len() >= 2);
        for (start, end) in &chunks {
            assert!(end - start <= 221, "chunk of {} chars", end - start);
        }
    }

    #[test]
    fn narration_heading_alone_then_body() {
        let text = "Chapter 3\nThe rain fell for days and the river rose.";
        let chunks = split_for_narration(text);
        assert_eq!(chunks.len(), 2);
        let first: String = text
            .chars()
            .skip(chunks[0].0)
            .take(chunks[0].1 - chunks[0].0)
            .collect();
        assert!(first.contains("Chapter 3"));
    }
}
