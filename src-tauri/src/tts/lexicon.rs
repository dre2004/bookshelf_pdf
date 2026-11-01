//! Supplemental pronunciation lexicon.
//!
//! The CMU dictionary lacks many derivational and technical words
//! ("accessors", "vectorized", "endianness"), and the phonemizer's
//! fallback then spells them letter by letter. Words the dictionary
//! resolves fine never reach this module. For the rest:
//!
//! 1. A curated table handles stress-shifting or irregular words
//!    ("executable") and common vocabulary CMU simply lacks.
//! 2. Suffix rules derive the rest from a stem the dictionary (or the
//!    curated table) knows: stem pronunciation + the suffix's phonemes,
//!    with allomorph selection (d/t/ɪd, s/z/ɪz) driven by the stem's
//!    final phoneme. Derivation keeps the stem's stress, which is
//!    correct for the overwhelming majority of technical derivations
//!    (assignable, composable, vectorized); the exceptions are curated.
//!
//! Every phoneme string uses the phonemizer's own espeak-IPA notation.

use piper_plus_g2p::Phonemizer as _;
use piper_plus_g2p::english::EnglishPhonemizer;

/// Curated pronunciations for words derivation can't build correctly.
/// Checked before suffix rules; also consulted as a stem source, so
/// inflections of these words ("dereferencing") derive normally.
const SUPPLEMENTS: &[(&str, &str)] = &[
    ("executable", "ɪɡzˈɛkjətəbəl"),
    ("executables", "ɪɡzˈɛkjətəbəlz"),
    ("initializer", "ɪnˈɪʃəlaɪzɚ"),
    ("initializers", "ɪnˈɪʃəlaɪzɚz"),
    ("endianness", "ˈɛndiənnəs"),
    ("bitwise", "bˈɪtwaɪz"),
    ("recursion", "ɹɪkˈɜːɹʒən"),
    ("dereference", "diːɹˈɛfɚəns"),
    ("toolchain", "tˈuːltʃeɪn"),
    ("toolchains", "tˈuːltʃeɪnz"),
    ("whitespace", "wˈaɪtspeɪs"),
    ("hexadecimal", "hˌɛksədˈɛsəməl"),
    ("lowercase", "lˈoʊɚkeɪs"),
    ("uppercase", "ˈʌpɚkeɪs"),
    ("metadata", "mˈɛtədeɪtə"),
    ("namespace", "nˈeɪmspeɪs"),
    ("operand", "ˈɑpɚənd"),
    ("operands", "ˈɑpɚəndz"),
    ("suffix", "sˈʌfɪks"),
    ("suffixes", "sˈʌfɪksɪz"),
    ("checksum", "tʃˈɛksʌm"),
    ("checksums", "tʃˈɛksʌmz"),
    ("callback", "kˈælbæk"),
    ("callbacks", "kˈælbækz"),
    ("boolean", "bˈuːliən"),
    ("booleans", "bˈuːliənz"),
    ("runtime", "ɹˈʌntaɪm"),
    ("runtimes", "ɹˈʌntaɪmz"),
    ("sanitizer", "sˈænɪtaɪzɚ"),
    ("sanitizers", "sˈænɪtaɪzɚz"),
    ("wraparound", "ɹˈæpəɹaʊnd"),
    ("workflow", "wˈɜːkflˈoʊ"),
    ("workflows", "wˈɜːkflˈoʊz"),
    ("timestamp", "tˈaɪmstæmp"),
    ("timestamps", "tˈaɪmstæmps"),
    // Audited stragglers: stress moves (algorithmic) or the stem is
    // irregular (appendices), so the rules can't build them.
    ("algorithmic", "ˌælɡɚˈɪðmɪk"),
    ("appendices", "əpˈɛndɪsiz"),
    ("arities", "əɹˈɪtiz"),
    ("arithmetically", "ˌæɹɪθˈmɛtɪkli"),
    ("affordance", "əfˈɔɹdəns"),
    ("affordances", "əfˈɔɹdənsɪz"),
    ("accesses", "əksˈɛsɪz"),
    // Reported from the book: CMU lacks both, and they letter-spell
    // ("a-s-s-o-c-i-a-t-i-v-i-t-y").
    ("associative", "əsˈoʊsiˌeɪtɪv"),
    ("associativity", "əsˌoʊsiəˈtɪvəti"),
];

/// espeak-IPA appended to a stem's pronunciation for each derivational
/// suffix. Ordered longest-first. Kept to suffixes whose attachment does
/// not move the stem's stress; stress-shifters (executable) are curated.
const SUFFIX_RULES: &[(&str, &str)] = &[
    // -ization / -izable / -ized / -izer / -izes / -ize: the technical
    // workhorses (vectorization, vectorizable, vectorized, serializer).
    ("ization", "aɪzeɪʃən"),
    ("izable", "aɪzəbəl"),
    ("izers", "aɪzɚz"),
    ("izer", "aɪzɚ"),
    ("ized", "aɪzd"),
    ("izes", "aɪzɪz"),
    ("ize", "aɪz"),
    // agentives and their plurals: accessor = access + or.
    ("ors", "ɚz"),
    ("or", "ɚ"),
    ("ers", "ɚz"),
    // neutral nominal/adjectival suffixes.
    ("ness", "nəs"),
    ("ment", "mənt"),
    ("ful", "fəl"),
    // stress-keeping -able.
    ("able", "əbəl"),
    ("ism", "ɪzəm"),
    ("ist", "ɪst"),
];

/// The pronunciation for `word`, or None when neither the curated table
/// nor suffix rules can build one (the phonemizer's fallback stands).
/// `phonemize` pronounces stems and must reject stems it doesn't know
/// (return None rather than letter-spelling).
pub(crate) fn resolve<F>(word: &str, mut phonemize: F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    let word = word
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_ascii_lowercase();
    if word.len() < 5 {
        return None;
    }
    if let Some((_, ipa)) = SUPPLEMENTS.iter().find(|(w, _)| *w == word) {
        return Some((*ipa).to_string());
    }

    // Derivational suffixes.
    for (suffix, suffix_ipa) in SUFFIX_RULES {
        let Some(stem) = word.strip_suffix(suffix) else {
            continue;
        };
        if stem.len() < 4 {
            continue;
        }
        // Stem candidates: bare, and with a restored 'e' for e-dropping
        // verbs not already covered by a rule (comparable -> comparable).
        for candidate in [stem, &format!("{stem}e")] {
            if let Some(stem_ipa) = stem_pronunciation(candidate, &mut phonemize) {
                return Some(format!("{stem_ipa}{suffix_ipa}"));
            }
        }
    }

    // Basic inflections, so curated stems decline too
    // (dereferencing -> dereference + ing, initializing -> initialize +
    // ing). The dictionary's own morphological fallback already handles
    // these for dictionary stems, so this mostly fires when the stem
    // comes from the curated table or lost a final 'e'.
    for (suffix, kind) in [("ing", 'g'), ("ed", 'd'), ("s", 's')] {
        let Some(stem) = word.strip_suffix(suffix) else {
            continue;
        };
        if stem.len() < 4 {
            continue;
        }
        for candidate in [stem, &format!("{stem}e")] {
            let Some(stem_ipa) = stem_pronunciation(candidate, &mut phonemize) else {
                continue;
            };
            let ending = inflection_ipa(kind, stem_ipa.chars().last())?;
            return Some(format!("{stem_ipa}{ending}"));
        }
    }
    None
}

/// The allomorph for an inflection suffix given the stem's final phoneme
/// (single character, so affricates appear as their final element):
/// "ed" is ɪd after t/d, t after voiceless, d otherwise; "s" is ɪz after
/// sibilants, s after voiceless, z otherwise; "ing" is always ɪŋ.
fn inflection_ipa(kind: char, last_phoneme: Option<char>) -> Option<&'static str> {
    let last = last_phoneme?;
    let voiceless = matches!(last, 'p' | 't' | 'k' | 'f' | 'θ' | 's' | 'ʃ' | 'x');
    let sibilant = matches!(last, 's' | 'z' | 'ʃ' | 'ʒ');
    match kind {
        'g' => Some("ɪŋ"),
        'd' => Some(match last {
            't' | 'd' => "ɪd",
            _ if voiceless => "t",
            _ => "d",
        }),
        's' => Some(if sibilant {
            "ɪz"
        } else if voiceless {
            "s"
        } else {
            "z"
        }),
        _ => None,
    }
}

/// The pronunciation of a stem: curated table first (so inflections of
/// curated words derive), otherwise the phonemizer — which rejects
/// stems it would spell letter by letter.
fn stem_pronunciation<F>(stem: &str, phonemize: &mut F) -> Option<String>
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some((_, ipa)) = SUPPLEMENTS.iter().find(|(w, _)| *w == stem) {
        return Some((*ipa).to_string());
    }
    phonemize(stem)
}

/// True when the phonemizer's output for `word` is the letter-name
/// fallback rather than a real pronunciation: the fallback stresses
/// every letter-sized syllable, real words stress one or two.
pub(crate) fn spells_letter_by_letter(word: &str, ipa: &str) -> bool {
    if word.chars().count() < 6 {
        return false;
    }
    let stresses = ipa.matches('ˈ').count() + ipa.matches('ˌ').count();
    stresses * 2 >= word.chars().count()
}

/// Resolve against a real phonemizer instance.
pub(crate) fn resolve_with_phonemizer(word: &str, p: &EnglishPhonemizer) -> Option<String> {
    resolve(word, |stem| {
        let (tokens, _) = p.phonemize_with_prosody(stem).ok()?;
        let ipa = tokens.concat();
        (!spells_letter_by_letter(stem, &ipa)).then_some(ipa)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_dict<'a>(stems: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |stem: &str| {
            stems
                .iter()
                .find(|(w, _)| *w == stem)
                .map(|(_, ipa)| ipa.to_string())
        }
    }

    #[test]
    fn derives_suffixes_from_dictionary_stems() {
        let f = fake_dict(&[
            ("access", "əksˈɛs"),
            ("vector", "vˈɛktɚ"),
            ("serial", "sˈɪɹiəl"),
        ]);
        assert_eq!(
            resolve("accessors", &f),
            Some("əksˈɛsɚz".to_string()),
            "agentive plural"
        );
        assert_eq!(resolve("accessor", &f), Some("əksˈɛsɚ".to_string()));
        assert_eq!(
            resolve("vectorized", &f),
            Some("vˈɛktɚaɪzd".to_string()),
            "-ized from the bare stem"
        );
        assert_eq!(
            resolve("vectorization", &f),
            Some("vˈɛktɚaɪzeɪʃən".to_string())
        );
        assert_eq!(resolve("serializer", &f), Some("sˈɪɹiəlaɪzɚ".to_string()));
    }

    #[test]
    fn curated_words_resolve_directly_and_decline() {
        let f = fake_dict(&[]);
        assert_eq!(resolve("executable", &f), Some("ɪɡzˈɛkjətəbəl".to_string()));
        // Both reported from the book letter-spelling through the CMU
        // fallback; the curated rows pin the fix.
        assert_eq!(
            resolve("associativity", &f),
            Some("əsˌoʊsiəˈtɪvəti".to_string())
        );
        assert_eq!(
            resolve("associative", &f),
            Some("əsˈoʊsiˌeɪtɪv".to_string())
        );
        // Inflections of curated words derive through the table with
        // allomorph selection on the stem's final phoneme.
        assert_eq!(
            resolve("dereferencing", &f),
            Some("diːɹˈɛfɚənsɪŋ".to_string())
        );
        // Stem ends in 's' (voiceless): "ed" -> t.
        assert_eq!(
            resolve("dereferenced", &f),
            Some("diːɹˈɛfɚənst".to_string())
        );
        // Stem ends in a sibilant: "s" -> ɪz.
        assert_eq!(
            resolve("dereferences", &f),
            Some("diːɹˈɛfɚənsɪz".to_string())
        );
    }

    #[test]
    fn short_and_unknown_words_return_none() {
        let f = fake_dict(&[("access", "əksˈɛs")]);
        assert_eq!(resolve("cat", &f), None);
        assert_eq!(resolve("xyzzyx", &f), None);
        // Unknown stem: nothing to build from.
        assert_eq!(resolve("qwertyors", &f), None);
    }

    #[test]
    fn letter_spelling_detector() {
        assert!(spells_letter_by_letter(
            "executable",
            "ˈiːˈɛksˈiːsˈiːjˈuːtˈiːˈeɪbˈiːˈɛlˈiː"
        ));
        assert!(!spells_letter_by_letter("executable", "ɪɡzˈɛkjətəbəl"));
        assert!(
            !spells_letter_by_letter("read", "ɹˈiːd"),
            "short words never flag"
        );
    }

    /// The real engine: derivations must produce IPA the tokenizer
    /// actually contains, and never the letter-name fallback.
    #[test]
    fn derivations_against_the_real_phonemizer() {
        let p = EnglishPhonemizer::new_bundled().expect("phonemizer");
        for word in [
            "accessors",
            "vectorized",
            "serializer",
            "initializers",
            "executable",
            "initializing",
        ] {
            let ipa = resolve_with_phonemizer(word, &p)
                .unwrap_or_else(|| panic!("{word} should resolve"));
            assert!(
                !spells_letter_by_letter(word, &ipa),
                "{word} resolved to letter names: {ipa}"
            );
        }
        assert_eq!(resolve_with_phonemizer("qwertyuiop", &p), None);
    }
}
