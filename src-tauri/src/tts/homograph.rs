//! Heteronym (homograph) disambiguation for the G2P front-end.
//!
//! English spellings map to several pronunciations and the pronunciation
//! dictionary picks one — often the wrong one for the common reading
//! ("wound" defaults to rhyming-with-"sound", "lead" to the metal). These
//! rules override the dictionary when local context makes the intent
//! unambiguous, and deliberately do nothing otherwise: a word read
//! consistently wrong beats a coin flip.
//!
//! Extend by adding rows to [`ENTRIES`]; a row is data, not a code path.
//! Phoneme strings are IPA in the phonemizer's own notation (the
//! phonemes_use_the_phonemizer_alphabet test pins the alphabet in use).

/// How to say an ambiguous word when a rule matches.
#[derive(Debug, PartialEq)]
pub(crate) enum Pronunciation {
    /// Say a differently-spelled word the dictionary already knows
    /// ("read" -> "reed").
    Respell(&'static str),
    /// Say exactly these phonemes, skipping the dictionary — used when the
    /// other reading has no respelling the dictionary knows ("record" the
    /// noun vs the verb differ only by stress).
    Phonemes(&'static str),
}

/// Where a word sits relative to its neighbors.
pub(crate) enum Context {
    /// First word of a sentence (or of the spoken piece).
    SentenceStart,
    /// Immediately preceded by one of these words (lowercase, apostrophes
    /// kept as in "don't").
    After(&'static [&'static str]),
    /// Immediately followed by one of these words — for cases the previous
    /// word can't settle ("Two tracks lead through": any noun can be a
    /// plural subject, but "lead through" is only ever the verb).
    FollowedBy(&'static [&'static str]),
}

/// Preceders that make a following word a verb: infinitive "to", modals,
/// negated auxiliaries and pronoun subjects ("they object", "to record").
const VERB_PRECEDERS: &[&str] = &[
    "to",
    "will",
    "would",
    "can",
    "could",
    "should",
    "shall",
    "may",
    "might",
    "must",
    "let's",
    "lets",
    "i",
    "you",
    "we",
    "they",
    "he",
    "she",
    "it",
    "who",
    "don't",
    "doesn't",
    "didn't",
    "won't",
    "can't",
    "couldn't",
    "shouldn't",
    "wouldn't",
];

/// Preceders that make a following word a noun (or noun-like adjective):
/// determiners, possessives and quantifiers ("a record", "the wound",
/// "the whole subject").
const DETERMINER_PRECEDERS: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "my", "your", "his", "her", "its", "our",
    "their", "no", "every", "each", "any", "some", "another", "whole",
];

/// Intensifiers before participial adjectives ("a very learned
/// professor", "a most blessed day").
const DEGREE_PRECEDERS: &[&str] = &["very", "most", "so"];

/// Preceders that make "read" present tense (rhymes with "reed"):
/// infinitives, modals, question words and common verb collocations.
const READ_PRESENT_PRECEDERS: &[&str] = &[
    "to", "will", "would", "can", "could", "should", "shall", "may", "might", "must", "let's",
    "lets", "want", "wants", "wanted", "going", "about", "how", "when", "where", "why", "what",
    "who", "if", "unless", "until", "before", "after", "once", "get", "gets", "help", "helps",
    "love", "loves", "like", "likes",
];

pub(crate) struct Entry {
    pub word: &'static str,
    pub rules: &'static [(Context, Pronunciation)],
}

/// The disambiguation table. Rule order within an entry is evaluation
/// order; first match wins. Absence of a matching rule means "keep the
/// dictionary default" — the only safe fallback.
pub(crate) const ENTRIES: &[Entry] = &[
    // --- heteronym pairs ---
    Entry {
        word: "read",
        rules: &[
            // Imperatives open sentences: "Read Part 0 once, end to end."
            (Context::SentenceStart, Pronunciation::Respell("reed")),
            (
                Context::After(READ_PRESENT_PRECEDERS),
                Pronunciation::Respell("reed"),
            ),
            // Past tense/participle ("have read") is the dictionary default.
        ],
    },
    Entry {
        word: "lead",
        rules: &[
            (
                Context::After(VERB_PRECEDERS),
                // Present-tense verb rhymes with "reed"; the dictionary's
                // default is the metal.
                Pronunciation::Phonemes("lˈiːd"),
            ),
            (
                Context::FollowedBy(&["through", "us", "them", "him", "her"]),
                // "Two tracks lead through this book": any noun can be a
                // plural subject, but "lead through" / "lead us" are only
                // ever the verb.
                Pronunciation::Phonemes("lˈiːd"),
            ),
        ],
    },
    Entry {
        word: "live",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb ("to live") rhymes with "give"; the dictionary's
            // default is the adjective ("a live wire").
            Pronunciation::Phonemes("lˈɪv"),
        )],
    },
    Entry {
        word: "wind",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The noun (moving air) rhymes with "pinned"; the dictionary's
            // default is the verb ("to wind" a clock).
            Pronunciation::Phonemes("wˈɪnd"),
        )],
    },
    Entry {
        word: "wound",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The injury rhymes with "mooned"; the dictionary's default is
            // the past of "wind". "She wound the clock" stays the default.
            Pronunciation::Phonemes("wˈuːnd"),
        )],
    },
    Entry {
        word: "close",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb has a voiced final consonant ("close the door").
            Pronunciation::Phonemes("klˈoʊz"),
        )],
    },
    Entry {
        word: "use",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb ("use the tool") has a voiced final consonant.
            Pronunciation::Phonemes("jˈuːz"),
        )],
    },
    Entry {
        word: "house",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb ("to house") has a voiced final consonant.
            Pronunciation::Phonemes("hˈaʊz"),
        )],
    },
    Entry {
        word: "dove",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // Past of "dive"; the dictionary's default is the bird.
            Pronunciation::Phonemes("dˈoʊv"),
        )],
    },
    // --- noun/verb stress pairs (the dictionary keeps one stress; a rule
    //     supplies the other) ---
    Entry {
        word: "record",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The noun stresses the first syllable (REK-ord); the
            // dictionary's default is the verb (re-CORD).
            Pronunciation::Phonemes("ɹˈɛkɚd"),
        )],
    },
    Entry {
        word: "present",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb stresses the second syllable (pre-ZENT).
            Pronunciation::Phonemes("pɹɪˈzɛnt"),
        )],
    },
    Entry {
        word: "object",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb stresses the second syllable (ob-JECT).
            Pronunciation::Phonemes("əbdʒˈɛkt"),
        )],
    },
    Entry {
        word: "produce",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The noun stresses the first syllable (PRO-duce, vegetables).
            Pronunciation::Phonemes("pɹˈoduːs"),
        )],
    },
    Entry {
        word: "refuse",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The noun (garbage) stresses the first syllable.
            Pronunciation::Phonemes("ɹˈɛfjuːs"),
        )],
    },
    Entry {
        word: "conduct",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The noun stresses the first syllable (CON-duct).
            Pronunciation::Phonemes("kˈɑndʌkt"),
        )],
    },
    Entry {
        word: "contract",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb stresses the second syllable (con-TRACT).
            Pronunciation::Phonemes("kəntɹˈækt"),
        )],
    },
    Entry {
        word: "progress",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb stresses the second syllable (pro-GRESS).
            Pronunciation::Phonemes("pɹəɡɹˈɛs"),
        )],
    },
    Entry {
        word: "project",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb stresses the second syllable (pro-JECT).
            Pronunciation::Phonemes("pɹədˈʒɛkt"),
        )],
    },
    Entry {
        word: "subject",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The noun stresses the first syllable (SUB-ject).
            Pronunciation::Phonemes("sˈʌbdʒɪkt"),
        )],
    },
    Entry {
        word: "estimate",
        rules: &[(
            Context::After(VERB_PRECEDERS),
            // The verb ends in a long-a diphthong (ES-ti-mate the cost).
            Pronunciation::Phonemes("ˈɛstəmeɪt"),
        )],
    },
    Entry {
        word: "separate",
        rules: &[(
            Context::After(DETERMINER_PRECEDERS),
            // The adjective ends in a schwa (SEP-uh-ret).
            Pronunciation::Phonemes("sˈɛpɚɪt"),
        )],
    },
    // --- participial adjectives: "-ed" spoken as its own syllable ---
    Entry {
        word: "learned",
        rules: &[(
            Context::After(DEGREE_PRECEDERS),
            // "a very learned professor": LEARN-ed. Determiners alone are
            // ambiguous, so only degree words trigger this.
            Pronunciation::Phonemes("lˈɜːnɪd"),
        )],
    },
    Entry {
        word: "blessed",
        rules: &[(
            Context::After(DEGREE_PRECEDERS),
            Pronunciation::Phonemes("blˈɛsɪd"),
        )],
    },
    Entry {
        word: "aged",
        rules: &[(
            Context::After(DEGREE_PRECEDERS),
            Pronunciation::Phonemes("ˈeɪdʒɪd"),
        )],
    },
];

/// The pronunciation for `words[index]`, if a rule matches with
/// confidence. Words are the raw whitespace tokens of the piece.
pub(crate) fn resolve(words: &[&str], index: usize) -> Option<&'static Pronunciation> {
    let word = clean(words.get(index)?);
    let entry = ENTRIES.iter().find(|e| e.word.eq_ignore_ascii_case(word))?;
    let raw_prev = index.checked_sub(1).map(|i| words[i]);
    let prev = raw_prev.map(clean);
    let at_sentence_start = match raw_prev {
        // A preceding sentence terminator (or a bare punctuation token,
        // which cleans to empty) means this word opens a sentence. The
        // terminator test uses the raw token — clean() strips it.
        Some(p) => {
            let cleaned = clean(p);
            cleaned.is_empty() || p.ends_with('.') || p.ends_with('!') || p.ends_with('?')
        }
        None => true,
    };
    entry.rules.iter().find_map(|(context, pronunciation)| {
        let matched =
            match context {
                Context::SentenceStart => at_sentence_start,
                Context::After(list) => prev.is_some_and(|p| {
                    list.iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(p))
                }),
                Context::FollowedBy(list) => words
                    .get(index + 1)
                    .map(|next| clean(next))
                    .is_some_and(|next| {
                        list.iter()
                            .any(|candidate| candidate.eq_ignore_ascii_case(next))
                    }),
            };
        matched.then_some(pronunciation)
    })
}

/// Strips surrounding punctuation but keeps apostrophes ("don't",
/// "let's"). Returns the empty string for pure-punctuation tokens.
fn clean(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(sentence: &str, word: &str) -> Option<&'static Pronunciation> {
        let tokens: Vec<&str> = sentence.split_whitespace().collect();
        let index = tokens
            .iter()
            .position(|t| clean(t).eq_ignore_ascii_case(word))
            .expect("word not in sentence");
        resolve(&tokens, index)
    }

    #[test]
    fn read_is_reed_in_imperatives_and_after_infinitives() {
        // The sentence that prompted the feature.
        assert!(matches!(
            spoken("Read Part 0 once, end to end", "read"),
            Some(Pronunciation::Respell("reed"))
        ));
        assert!(matches!(
            spoken("You should read it carefully.", "read"),
            Some(Pronunciation::Respell("reed"))
        ));
        assert!(matches!(
            spoken("How to read this book", "read"),
            Some(Pronunciation::Respell("reed"))
        ));
    }

    #[test]
    fn read_stays_default_after_have_and_mid_sentence() {
        // "have read" (past participle) and narrative past keep the
        // dictionary's default.
        assert_eq!(spoken("They have read the chapter.", "read"), None);
        assert_eq!(spoken("I read it yesterday.", "read"), None);
    }

    #[test]
    fn sentence_start_detected_after_terminators() {
        // "Read" opens the second sentence of the group.
        assert!(matches!(
            spoken("It was late. Read the appendix.", "read"),
            Some(Pronunciation::Respell("reed"))
        ));
    }

    #[test]
    fn stress_pairs_flip_on_context() {
        assert!(matches!(
            spoken("the record", "record"),
            Some(Pronunciation::Phonemes("ɹˈɛkɚd"))
        ));
        assert_eq!(spoken("they will record", "record"), None);
        assert!(matches!(
            spoken("they object", "object"),
            Some(Pronunciation::Phonemes("əbdʒˈɛkt"))
        ));
        assert_eq!(spoken("an object", "object"), None);
        assert!(matches!(
            spoken("to present", "present"),
            Some(Pronunciation::Phonemes("pɹɪˈzɛnt"))
        ));
    }

    #[test]
    fn heteronym_pairs_flip_on_context() {
        assert!(matches!(
            spoken("the wind", "wind"),
            Some(Pronunciation::Phonemes("wˈɪnd"))
        ));
        assert_eq!(spoken("to wind", "wind"), None);
        assert!(matches!(
            spoken("a wound", "wound"),
            Some(Pronunciation::Phonemes("wˈuːnd"))
        ));
        // "She wound the clock" (past of wind) keeps the default.
        assert_eq!(spoken("she wound", "wound"), None);
        assert!(matches!(
            spoken("they close", "close"),
            Some(Pronunciation::Phonemes("klˈoʊz"))
        ));
        assert!(matches!(
            spoken("to live", "live"),
            Some(Pronunciation::Phonemes("lˈɪv"))
        ));
        assert!(matches!(
            spoken("we use", "use"),
            Some(Pronunciation::Phonemes("jˈuːz"))
        ));
        assert!(matches!(
            spoken("she dove", "dove"),
            Some(Pronunciation::Phonemes("dˈoʊv"))
        ));
        assert_eq!(spoken("the dove", "dove"), None);
    }

    #[test]
    fn punctuation_around_words_is_ignored() {
        assert!(matches!(
            spoken("\"Read this.\"", "read"),
            Some(Pronunciation::Respell("reed"))
        ));
        assert!(matches!(
            spoken("after the record, things changed", "record"),
            Some(Pronunciation::Phonemes("ɹˈɛkɚd"))
        ));
    }

    #[test]
    fn audit_findings_from_the_real_book() {
        // "Two tracks lead through this book": noun subject, verb sense.
        assert!(matches!(
            spoken("Two tracks lead through this book", "lead"),
            Some(Pronunciation::Phonemes("lˈiːd"))
        ));
        // "a lead pipe" stays the metal (noun + noun compound).
        assert_eq!(spoken("a lead pipe", "lead"), None);
        // "the whole subject": quantifier between determiner and noun.
        assert!(matches!(
            spoken("the whole subject", "subject"),
            Some(Pronunciation::Phonemes("sˈʌbdʒɪkt"))
        ));
        // "declarations that read inside-out" is genuinely ambiguous
        // ("the book that I read" is equally grammatical): keep default.
        assert_eq!(spoken("declarations that read inside-out", "read"), None);
    }

    #[test]
    fn unambiguous_words_are_untouched() {
        assert_eq!(spoken("the book", "book"), None);
        assert_eq!(spoken("to write", "write"), None);
    }

    /// Documents the dictionary defaults these rules override, and pins
    /// the phoneme alphabet: every character the table emits must occur
    /// in real phonemizer output, or the tokenizer would silently drop
    /// it.
    #[test]
    fn phonemes_use_the_phonemizer_alphabet() {
        use piper_plus_g2p::Phonemizer as _;
        let p = piper_plus_g2p::english::EnglishPhonemizer::new_bundled().expect("phonemizer");
        let defaults: &[(&str, &str)] = &[
            ("read", "ɹˈɛd"),
            ("record", "ɹəkˈɔːɹd"),
            ("wound", "wˈaʊnd"),
            ("separate", "sˈɛpɚˌeɪt"),
        ];
        for (word, expected) in defaults {
            let (tokens, _) = p.phonemize_with_prosody(word).expect("g2p");
            assert_eq!(
                &tokens.concat(),
                expected,
                "dictionary default changed for {word}"
            );
        }
        // Collect the alphabet from a broad pronunciation sweep of the
        // very words the table is about, then require every table char to
        // be present in it.
        let mut alphabet = String::new();
        for entry in ENTRIES {
            let (tokens, _) = p.phonemize_with_prosody(entry.word).expect("g2p");
            alphabet.push_str(&tokens.concat());
        }
        let (tokens, _) = p
            .phonemize_with_prosody("the quick brown fox jumps over the lazy dog")
            .expect("g2p");
        alphabet.push_str(&tokens.concat());
        for entry in ENTRIES {
            for (_, pronunciation) in entry.rules {
                if let Pronunciation::Phonemes(phonemes) = pronunciation {
                    for c in phonemes.chars() {
                        assert!(
                            alphabet.contains(c),
                            "phoneme char {c:?} in {phonemes:?} is not in the phonemizer alphabet"
                        );
                    }
                }
            }
        }
    }
}
