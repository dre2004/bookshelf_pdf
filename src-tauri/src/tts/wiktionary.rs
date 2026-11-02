//! Wiktionary-derived pronunciation table, bundled with the app.
//!
//! `en-US.tsv.gz` is distilled from kaikki.org's machine-readable English
//! Wiktionary extract by `scripts/extract-wiktionary-ipa.mjs` and refreshed
//! monthly by the lexicon workflow; a small seed ships in the repo so the
//! build never depends on the extraction having run. Lines are
//! `word<TAB>IPA` in the Kokoro tokenizer's symbol set.
//!
//! The table's job is the layer the CMU dictionary misses: acronyms and
//! recent coinages. All-caps tokens are consulted here *before* G2P
//! because the fallback's failures there are confidently wrong ("AI"
//! read as "eye") rather than detectably wrong.

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

const TABLE_GZ: &[u8] = include_bytes!("../../resources/pronunciation/en-US.tsv.gz");

/// Hand-verified rows for words the extract can't provide (verified
/// against the live pages, not the extraction) — consulted before the
/// table so they survive every refresh.
const CURATED: &[(&str, &str)] = &[
    // Wiktionary's LLM page carries its IPA in a form wiktextract drops.
    ("LLM", "elˈelem"),
];

fn curated(word: &str) -> Option<&'static str> {
    CURATED
        .iter()
        .find(|(entry, _)| *entry == word)
        .map(|(_, ipa)| *ipa)
}

fn table() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut text = String::new();
        if let Err(e) = flate2::read::GzDecoder::new(TABLE_GZ).read_to_string(&mut text) {
            // A corrupt bundled table must not take narration down; the
            // curated lexicon still answers.
            eprintln!("bundled pronunciation table: {e}");
        }
        parse(&text)
    })
}

/// Parses `word<TAB>IPA` lines; malformed lines are skipped.
pub(crate) fn parse(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (word, ipa) = line.split_once('\t')?;
            if word.is_empty() || ipa.is_empty() {
                return None;
            }
            Some((word.to_string(), ipa.to_string()))
        })
        .collect()
}

/// The token shape we look up: surrounding punctuation stripped, inner
/// apostrophes and hyphens kept.
fn clean_token(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .replace('’', "'")
}

/// All-caps shape: "LLM", "LLM's", "LLMs" (a trailing plural s may be
/// lowercase). Single letters are excluded — they are ordinary words
/// ("A", "I").
fn is_acronym(word: &str) -> bool {
    let mut letters: Vec<char> = word.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if matches!(letters.last(), Some('s' | 'S')) {
        letters.pop();
    }
    letters.len() >= 2 && letters.iter().all(|c| c.is_ascii_uppercase())
}

/// Plural/possessive allomorph: ɪz after a sibilant (including the
/// affricates ʃ/ʒ, which also ends tʃ/dʒ), z otherwise — the same rule
/// the curated lexicon's inflections apply.
fn plural_suffix(stem_ipa: &str) -> &'static str {
    match stem_ipa.chars().last() {
        Some('s' | 'z' | 'ʃ' | 'ʒ' | 'ʧ' | 'ʤ') => "ɪz",
        _ => "z",
    }
}

/// Strips a plural/possessive ending: "LLM's"/"LLMs'"/"LLMs" → "LLM".
fn strip_plural(word: &str) -> &str {
    word.strip_suffix("'s")
        .or_else(|| word.strip_suffix("s'"))
        .or_else(|| word.strip_suffix('s'))
        .unwrap_or(word)
}

/// Acronym resolution against a table: exact entry first ("LLM"), then a
/// lowercase fold ("RAG" → "rag", which is how the initialism is said),
/// then a plural/possessive strip ("LLMs"/"LLM's" → "LLM" + z). The exact
/// match ordering means real multi-letter acronyms are never mangled by
/// the strip once the full table knows them ("AWS" resolves as itself,
/// not as "AW" + z).
fn lookup_acronym(map: &HashMap<String, String>, token: &str) -> Option<String> {
    let cleaned = clean_token(token);
    if !is_acronym(&cleaned) {
        return None;
    }
    if let Some(ipa) = curated(&cleaned) {
        return Some(ipa.to_string());
    }
    if let Some(ipa) = map.get(&cleaned) {
        return Some(ipa.clone());
    }
    let lower = cleaned.to_lowercase();
    if let Some(ipa) = map.get(&lower) {
        return Some(ipa.clone());
    }
    let stem = strip_plural(&cleaned);
    if stem.len() >= 2 && stem != cleaned {
        let ipa = curated(stem)
            .map(str::to_string)
            .or_else(|| map.get(stem).cloned())
            .or_else(|| map.get(&stem.to_lowercase()).cloned())?;
        Some(format!("{ipa}{}", plural_suffix(&ipa)))
    } else {
        None
    }
}

/// The pronunciation for an all-caps token, or None. Consulted before
/// G2P so confidently-wrong fallbacks never reach the model.
pub(crate) fn acronym_ipa(token: &str) -> Option<String> {
    lookup_acronym(table(), token)
}

/// Table entry for a word the dictionary letter-spells: exact match
/// first, then a case fold. Unlike `acronym_ipa` this is only consulted
/// after a detectable failure, so normal words never pay for it.
pub(crate) fn entry_ipa(word: &str) -> Option<String> {
    let cleaned = clean_token(word);
    if let Some(ipa) = curated(&cleaned) {
        return Some(ipa.to_string());
    }
    table()
        .get(&cleaned)
        .or_else(|| table().get(&cleaned.to_lowercase()))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> HashMap<String, String> {
        parse("AI\tˌeɪˈaɪ\nLLM\telˈelem\nrag\tɹæɡ\nIV\taɪˈviː\nUS\tˌjuːˈɛs\n")
    }

    #[test]
    fn parses_and_skips_malformed_lines() {
        let map = parse("a\tb\nbroken\n\t\nx\t");
        assert_eq!(map.get("a").map(String::as_str), Some("b"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn acronym_exact_match_wins() {
        assert_eq!(
            lookup_acronym(&map(), "AI").as_deref(),
            Some("ˌeɪˈaɪ"),
            "\"AI\" must not fall through to the aɪ fallback"
        );
        assert_eq!(
            lookup_acronym(&map(), "AI,").as_deref(),
            Some("ˌeɪˈaɪ"),
            "punctuated tokens are cleaned before lookup"
        );
    }

    #[test]
    fn acronym_case_folds_to_the_spoken_word() {
        assert_eq!(lookup_acronym(&map(), "RAG").as_deref(), Some("ɹæɡ"));
        // Roman numerals read the way people say them.
        assert_eq!(lookup_acronym(&map(), "IV").as_deref(), Some("aɪˈviː"));
    }

    #[test]
    fn acronym_plural_strips_and_conjugates() {
        assert_eq!(lookup_acronym(&map(), "LLMs").as_deref(), Some("elˈelemz"));
        assert_eq!(lookup_acronym(&map(), "LLM’s").as_deref(), Some("elˈelemz"));
        // Sibilant-final stems take ɪz, not z.
        assert_eq!(lookup_acronym(&map(), "US's").as_deref(), Some("ˌjuːˈɛsɪz"));
    }

    #[test]
    fn non_acronyms_are_not_looked_up() {
        for token in ["Chapter", "hello", "I", "A", "x", "Apple"] {
            assert!(
                lookup_acronym(&map(), token).is_none(),
                "{token} should not be treated as a table acronym"
            );
        }
    }

    #[test]
    fn quoted_acronyms_are_cleaned_not_rejected() {
        assert_eq!(lookup_acronym(&map(), "“LLM”").as_deref(), Some("elˈelem"));
    }

    #[test]
    fn bundled_table_loads_and_covers_the_pinned_words() {
        let table = table();
        assert!(table.contains_key("AI"), "table missing AI");
        assert!(
            table.contains_key("rag"),
            "table missing rag (the RAG fold)"
        );
        // LLM has no IPA in the extract; the curated override answers.
        assert!(!table.contains_key("LLM"));
        assert_eq!(acronym_ipa("LLM").as_deref(), Some("elˈelem"));
    }
}
