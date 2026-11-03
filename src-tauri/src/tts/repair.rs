//! Token repair for machine-shaped text in narrated pages.
//!
//! Technical books are full of identifiers, module paths, URLs and OCR
//! damage; none of it is dictionary vocabulary and all of it used to be
//! read letter by letter. This module turns machine tokens into speakable
//! words *before* phonemization:
//!
//! - `split_structural` — camelCase, snake_case, `::`/`/`/`.` and friends
//!   become word boundaries (`HttpResponse` → "Http Response",
//!   `Vec::new` → "Vec new", `lower_right` → "lower right").
//! - `classify_machine_text` — URLs read as "link"; permission strings,
//!   version numbers and hex hashes are dropped.
//! - `repair_ocr` — scanned books misread capital I as lowercase l
//!   (`anlnteger`); an l→I swap that produces known words is taken.
//! - `segment_glued` — spaceless concatenations of real words are split
//!   by exhaustive dictionary match (`youcanbuild` → "you can build").
//!
//! Dictionary-dependent repairs only run for tokens that would otherwise
//! letter-spell (decided by the caller), so real words are never touched.

/// How a machine-looking token should be narrated.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MachineText {
    /// Ordinary word: narrate as-is.
    Speak,
    /// Say the word "link" instead of the token.
    Link,
    /// Say nothing.
    Drop,
}

/// Separator characters that join words inside identifiers and paths.
/// Sentence punctuation is deliberately absent — hyphens and apostrophes
/// belong to real words ("cross-references", "don't"), and trailing
/// periods are sentence terminators the phonemizer needs.
const SEPARATORS: &[char] = &[
    '_', ':', '/', '\\', '.', '»', '«', '<', '>', '(', ')', '[', ']', '{', '}', ',', ';', '=', '+',
    '&', '|', '#', '@', '$', '%', '^', '~', '*', '"',
];

/// Is this char position a split point? Separator characters, plus a
/// hyphen only when doubled ("--" is a dash, "cross-references" is a
/// word).
fn separator_at(chars: &[char], i: usize) -> bool {
    let c = chars[i];
    SEPARATORS.contains(&c)
        || (c == '-' && (chars.get(i + 1) == Some(&'-') || (i > 0 && chars[i - 1] == '-')))
}

/// Splits a token on identifier separators and case boundaries. All-caps
/// tokens are left alone (acronyms), and a leading capital is not a
/// boundary ("Word" stays "Word"). A trailing digit run splits off
/// (`array1` → "array 1") so identifiers with counters read naturally.
/// Leading/trailing identifier punctuation is dropped first so it cannot
/// block the split ("(index2:" splits); a trailing sentence period or
/// comma stays attached to the final piece.
pub(crate) fn split_structural(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    if chars.is_empty() {
        return vec![token.to_string()];
    }
    // Trim separator chars from both ends, but keep a trailing '.' or ','
    // (sentence punctuation) attached.
    let keep_trailing_punct = matches!(chars.last(), Some('.' | ','));
    let mut start = 0;
    while start < chars.len() && separator_at(&chars, start) {
        start += 1;
    }
    let mut end = chars.len();
    while end > start
        && separator_at(&chars, end - 1)
        && !(end == chars.len() && keep_trailing_punct)
    {
        end -= 1;
    }
    if end == chars.len() && keep_trailing_punct {
        // Keep the punctuation in the core; the final-piece rule below
        // re-attaches it.
        end = chars.len();
    }
    if start >= end {
        return vec![token.to_string()];
    }
    let core: Vec<char> = chars[start..end].to_vec();

    let has_separator = (0..core.len()).any(|i| separator_at(&core, i));
    let letters: Vec<char> = core
        .iter()
        .filter(|c| c.is_ascii_alphabetic())
        .copied()
        .collect();
    let all_caps = letters.len() >= 2 && letters.iter().all(|c| c.is_ascii_uppercase());
    let camel = !all_caps && core.iter().skip(1).any(|c| c.is_ascii_uppercase());
    let trailing_digits = {
        let last_digit = core.iter().rposition(|c| c.is_ascii_digit());
        let run_start = last_digit.map(|last| {
            let mut first = last;
            while first > 0 && core[first - 1].is_ascii_digit() {
                first -= 1;
            }
            first
        });
        match run_start {
            Some(split)
                if split >= 3
                    && core[..split].iter().all(|c| c.is_ascii_alphabetic())
                    && !has_separator
                    && !camel =>
            {
                Some(split)
            }
            _ => None,
        }
    };
    if !has_separator && !camel && trailing_digits.is_none() {
        // Nothing structural: hand the token back with its punctuation.
        return vec![token.to_string()];
    }

    let (head, tail): (Vec<char>, Option<Vec<char>>) = match trailing_digits {
        Some(split) => (core[..split].to_vec(), Some(core[split..].to_vec())),
        None => (core.clone(), None),
    };

    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, &c) in head.iter().enumerate() {
        let prev = if i > 0 { Some(head[i - 1]) } else { None };
        let next = head.get(i + 1).copied();
        // A sentence-final period/comma stays attached to its word —
        // only interior separators split ("Cargo.toml" splits, "etc."
        // does not).
        if (c == '.' || c == ',') && i + 1 == head.len() {
            current.push(c);
            break;
        }
        if let Some(p) = prev {
            let at_separator = separator_at(&head, i - 1) || separator_at(&head, i);
            // lower→Upper starts a new word ("aPoint"); Upper→Upper only
            // when an uppercase run ends and a real word follows — two or
            // more lowercase letters ("HTTPResponse" → HTTP|Response),
            // not a one-letter suffix ("URLs" stays whole).
            let at_case = c.is_ascii_uppercase()
                && (p.is_ascii_lowercase()
                    || (p.is_ascii_uppercase()
                        && next.is_some_and(|n| n.is_ascii_lowercase())
                        && head.get(i + 2).is_some_and(|n| n.is_ascii_lowercase())));
            if (at_separator || at_case) && !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
            }
        }
        if !separator_at(&head, i) {
            current.push(c);
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    if let Some(tail) = tail {
        // A kept trailing '.'/',' is already inside the tail slice
        // ("array1." → tail "1.") — nothing to re-attach.
        let tail: String = tail.into_iter().collect();
        if !tail.is_empty() {
            pieces.push(tail);
        }
    }
    // When the trailing punctuation stayed inside the head, the loop's
    // final-piece rule has already attached it.
    let pieces: Vec<String> = pieces.into_iter().filter(|p| !p.is_empty()).collect();
    if pieces.is_empty() {
        vec![token.to_string()]
    } else {
        pieces
    }
}

/// Extensions whose `name.ext` tokens are read as "link" — web domains
/// and source/config files.
const LINK_SUFFIXES: &[&str] = &[
    "com", "org", "net", "io", "dev", "edu", "gov", "co", "uk", "me", "txt", "toml", "json",
    "yaml", "yml", "xml", "css", "js", "ts", "py", "rb", "exe", "dll", "dylib", "pdf", "md",
    "lock", "html", "htm", "rs",
];

/// Classifies machine-shaped tokens. URLs, domains and source-file names
/// read as "link"; permission strings, versions, timestamps and hex
/// hashes are dropped; everything else is spoken.
pub(crate) fn classify_machine_text(token: &str) -> MachineText {
    if token.contains("://") || token.to_lowercase().starts_with("www.") {
        return MachineText::Link;
    }
    // word.tld / File.ext, allowing leading/trailing sentence punctuation.
    let bare = token.trim_matches(|c: char| !c.is_alphanumeric());
    if let Some((stem, ext)) = bare.rsplit_once('.')
        && !stem.is_empty()
            && stem
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            && (2..=5).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphabetic())
            && LINK_SUFFIXES.contains(&ext.to_lowercase().as_str())
        {
            return MachineText::Link;
        }
    // Unix permission strings: -rw-rw-r--, drwxr-xr-x.
    if bare.len() >= 6 && bare.chars().all(|c| "drwx-".contains(c)) {
        return MachineText::Drop;
    }
    // Semantic versions have at least two dot-separated numbers; a single
    // dot is a decimal ("3.14") and stays spoken. A leading "v" tags a
    // version ("v0.1.0") and is stripped first.
    let version_body = match bare.strip_prefix('v') {
        Some(body) if body.chars().next().is_some_and(|c| c.is_ascii_digit()) => body,
        _ => bare,
    };
    let dotted: Vec<&str> = version_body.split('.').collect();
    if dotted.len() >= 3
        && version_body
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        && dotted
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    {
        return MachineText::Drop;
    }
    // ISO timestamps: 2020-11-09T04:13:17Z.
    if bare.len() >= 10
        && bare.as_bytes().get(4) == Some(&b'-')
        && bare.as_bytes().get(7) == Some(&b'-')
        && bare[..4].chars().all(|c| c.is_ascii_digit())
    {
        return MachineText::Drop;
    }
    // Hex hashes (git, sha1): 7-40 hex chars with at least one a-f.
    if (7..=40).contains(&bare.len())
        && bare.chars().all(|c| c.is_ascii_hexdigit())
        && bare.chars().any(|c| c.is_ascii_alphabetic())
    {
        return MachineText::Drop;
    }
    MachineText::Speak
}

/// OCR repair for scanned books: the scanner reads capital I as lowercase
/// l. If replacing one or all of the token's `l`s with `I`s yields pieces
/// that are all known words after case splitting (`anlnteger` → an
/// Integer), the repaired form is returned.
pub(crate) fn repair_ocr(token: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
    if !token.contains('l') {
        return None;
    }
    let mut candidates: Vec<String> = vec![token.replace('l', "I")];
    let chars: Vec<char> = token.chars().collect();
    let l_count = chars.iter().filter(|c| **c == 'l').count();
    if l_count > 6 {
        return None; // bounded combinatorics
    }
    for skip in 0..l_count {
        let mut chars = chars.clone();
        let mut seen = 0;
        for c in chars.iter_mut() {
            if *c == 'l' {
                if seen == skip {
                    *c = 'I';
                    break;
                }
                seen += 1;
            }
        }
        candidates.push(chars.into_iter().collect());
    }
    candidates.into_iter().find(|candidate| {
        let pieces = split_structural(candidate);
        pieces.len() >= 2
            && pieces.iter().all(|piece| {
                let bare = piece.trim_matches(|c: char| !c.is_alphanumeric());
                bare.chars().count() >= 2 && known(&bare.to_lowercase())
            })
    })
}

/// Splits a spaceless concatenation of known words into those words,
/// covering the whole token (`youcanbuild` → "you can build"). Prefers
/// the fewest parts; single-letter parts are allowed only for "a".
pub(crate) fn segment_glued(token: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() < 5 || chars.len() > 30 {
        return None;
    }
    // best[i] = (fewest parts covering chars[..i], split index of the
    // last part).
    let mut best: Vec<Option<(usize, usize)>> = vec![None; chars.len() + 1];
    best[0] = Some((0, 0));
    for end in 1..=chars.len() {
        let mut winner: Option<(usize, usize)> = None;
        for start in (0..end).rev() {
            if best[start].is_none() {
                continue;
            }
            if end - start > 24 {
                break;
            }
            let part: String = chars[start..end].iter().collect();
            let bare = part
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            let part_len = bare.chars().count();
            if (part_len == 1 && bare == "a") || (part_len >= 2 && known(&bare)) {
                let candidate = (best[start].unwrap().0 + 1, start);
                if winner.is_none() || candidate.0 < winner.unwrap().0 {
                    winner = Some(candidate);
                }
            }
        }
        best[end] = winner;
    }
    let mut index = chars.len();
    let mut parts: Vec<String> = Vec::new();
    while index > 0 {
        let (_, prev) = best[index]?;
        parts.push(chars[prev..index].iter().collect());
        index = prev;
    }
    if parts.len() < 2 {
        return None;
    }
    parts.reverse();
    Some(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(word: &str) -> bool {
        [
            "an", "integer", "file", "in", "small", "you", "can", "build", "follow", "me", "walk",
            "back", "debug", "info", "docker", "hub",
        ]
        .contains(&word)
    }

    #[test]
    fn splits_camel_case_and_paths() {
        assert_eq!(
            split_structural("HttpResponse"),
            vec!["Http".to_string(), "Response".to_string()]
        );
        assert_eq!(
            split_structural("aPoint"),
            vec!["a".to_string(), "Point".to_string()]
        );
        assert_eq!(
            split_structural("Vec::new"),
            vec!["Vec".to_string(), "new".to_string()]
        );
        assert_eq!(
            split_structural("lower_right"),
            vec!["lower".to_string(), "right".to_string()]
        );
        assert_eq!(
            split_structural("Cargo.toml"),
            vec!["Cargo".to_string(), "toml".to_string()]
        );
        assert_eq!(
            split_structural("Complex<f64"),
            vec!["Complex".to_string(), "f64".to_string()]
        );
        assert_eq!(
            split_structural("Object»copy"),
            vec!["Object".to_string(), "copy".to_string()]
        );
        assert_eq!(
            split_structural("array1"),
            vec!["array".to_string(), "1".to_string()]
        );
        assert_eq!(
            split_structural("HTTPResponse"),
            vec!["HTTP".to_string(), "Response".to_string()]
        );
        // An uppercase run flowing into a suffix stays whole.
        assert_eq!(split_structural("URLs"), vec!["URLs".to_string()]);
    }

    #[test]
    fn real_words_and_acronyms_are_left_alone() {
        assert_eq!(
            split_structural("cross-references"),
            vec!["cross-references".to_string()]
        );
        assert_eq!(split_structural("don't"), vec!["don't".to_string()]);
        assert_eq!(split_structural("HTTP"), vec!["HTTP".to_string()]);
        assert_eq!(split_structural("UTF-8"), vec!["UTF-8".to_string()]);
        // Sentence-final periods stay attached: not an interior separator.
        assert_eq!(split_structural("word."), vec!["word.".to_string()]);
    }

    #[test]
    fn machine_text_is_classified() {
        assert_eq!(
            classify_machine_text("https://aqicn.org/city/barcelona"),
            MachineText::Link
        );
        assert_eq!(classify_machine_text("squeaksource.com"), MachineText::Link);
        assert_eq!(classify_machine_text("Cargo.toml"), MachineText::Link);
        assert_eq!(classify_machine_text("drwxrwxr-x"), MachineText::Drop);
        assert_eq!(classify_machine_text("-rw-rw-r--"), MachineText::Drop);
        assert_eq!(classify_machine_text("v0.1.0"), MachineText::Drop);
        assert_eq!(classify_machine_text("0.1.0"), MachineText::Drop);
        assert_eq!(
            classify_machine_text("2020-11-09T04:13:17Z"),
            MachineText::Drop
        );
        assert_eq!(classify_machine_text("deadbeefdead"), MachineText::Drop);
        assert_eq!(classify_machine_text("struct"), MachineText::Speak);
        assert_eq!(classify_machine_text("e.g"), MachineText::Speak);
        // Decimals are prose, not versions.
        assert_eq!(classify_machine_text("3.14"), MachineText::Speak);
        // Plain digit runs are not hashes.
        assert_eq!(classify_machine_text("1234567"), MachineText::Speak);
    }

    #[test]
    fn ocr_damage_is_repaired_by_l_to_i_swap() {
        assert_eq!(
            repair_ocr("anlnteger", &known).as_deref(),
            Some("anInteger")
        );
        assert_eq!(repair_ocr("fileln", &known).as_deref(), Some("fileIn"));
        // Nothing to repair comes back unchanged.
        assert_eq!(repair_ocr("println", &known), None);
    }

    #[test]
    fn glued_words_are_segmented() {
        assert_eq!(
            segment_glued("youcanbuild", &known).as_deref(),
            Some("you can build")
        );
        assert_eq!(
            segment_glued("followme", &known).as_deref(),
            Some("follow me")
        );
        assert_eq!(
            segment_glued("debuginfo", &known).as_deref(),
            Some("debug info")
        );
        // A coverable pair segments; an uncoverable word stays whole.
        assert_eq!(
            segment_glued("dockerhub", &known).as_deref(),
            Some("docker hub")
        );
        assert_eq!(segment_glued("structure", &known), None);
    }
}
