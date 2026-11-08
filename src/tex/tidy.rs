//! Tier 2: improvements made without asking, on a fresh draft or on a song
//! from the songbook. Each rule finds byte spans to replace; nothing else in
//! the song is touched.

use super::{PartSpan, Scan, code_of, scan};
use std::ops::Range;

/// One replacement, and what to tell the user about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub span: Range<usize>,
    pub replacement: String,
    /// The 1-based line it is on.
    pub line: usize,
    pub description: String,
    /// How many lines fewer the song has once it is made.
    pub removed_lines: usize,
}

/// Tidies a song: every rule, every change applied.
///
/// Chords move first, and the choruses are compared afterwards: a chorus
/// whose repeat had its chords placed right by hand only matches it once
/// the first one is placed right too. Tidying what this returns again
/// changes nothing. The changes are reported in the order they were made.
/// A move stays on its line, so the line numbers hold for both rounds.
pub fn tidy(text: &str) -> (String, Vec<Change>) {
    // A reminder typed by hand is the first thing to go: the chord moves
    // and the chorus comparison after it see the song as it will stay.
    let mut reminders = hand_written_reminders(text, &scan(text));
    reminders.sort_by_key(|c| c.span.start);
    let reminded = apply(text, &reminders);
    let text = reminded.as_str();

    let mut moves = chords_to_syllable_starts(text, &scan(text));
    moves.sort_by_key(|c| c.span.start);
    let moved = apply(text, &moves);

    let mut choruses = repeated_choruses(&moved, &scan(&moved));
    choruses.sort_by_key(|c| c.span.start);
    let tidied = apply(&moved, &choruses);

    // Removing a reminder takes its lines out; the later changes are
    // reported on the lines of the song as it was given.
    let mut later = moves;
    later.extend(choruses);
    {
        let original = text_before(&reminders);
        for change in &mut later {
            change.line = original(change.line);
        }
    }
    let mut changes = reminders;
    changes.extend(later);
    changes.sort_by_key(|c| c.line);
    (tidied, changes)
}

/// Maps a line of the song after `reminders` were applied back to the line
/// it was on before.
fn text_before(reminders: &[Change]) -> impl Fn(usize) -> usize + '_ {
    move |line| {
        let mut shift = 0;
        for reminder in reminders {
            let removed = reminder.removed_lines;
            // Where the reminder's replacement now stands.
            if reminder.line - shift < line {
                shift += removed;
            }
        }
        line + shift
    }
}

/// Applies changes that don't overlap, in order.
pub fn apply(text: &str, changes: &[Change]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for change in changes {
        debug_assert!(change.span.start >= at, "overlapping changes");
        out.push_str(&text[at..change.span.start]);
        out.push_str(&change.replacement);
        at = change.span.end;
    }
    out.push_str(&text[at..]);
    out
}

/// A chorus typed out again, word for word and chord for chord, becomes
/// `\insertchorus`, which reprints the chorus exactly as it was. (Checked by
/// typesetting both: the pages are identical, on A4 and A5.)
///
/// `\insertchorus` reprints the *latest* chorus, number and all, so a
/// chorus is only replaced when it matches the one right before it, with
/// the same number and the same switches after `\beginchorus`. Whatever
/// could make the latest chorus differ from what the song shows is reason
/// enough to leave the song alone: a chorus inside `\iflargepaper … \fi`
/// (which chorus is the latest depends on the paper), and chord memory
/// (`^`, `\memorize`, `\usechords`), which the two copies could replay
/// differently.
fn repeated_choruses(text: &str, scan: &Scan) -> Vec<Change> {
    let choruses: Vec<&PartSpan> = scan.parts.iter().filter(|p| p.kind == "chorus").collect();
    let conditional = choruses.iter().any(|p| p.conditional)
        || scan
            .chorus_repeats
            .iter()
            .any(|(_, conditional)| *conditional);
    if conditional {
        return Vec::new();
    }

    let mut changes = Vec::new();
    for pair in choruses.windows(2) {
        let (latest, repeat) = (pair[0], pair[1]);
        if !same_chorus(text, latest, repeat) {
            continue;
        }
        let replacement = match repeat.number {
            Some(n) => format!("\\insertchorus[{n}]"),
            None => "\\insertchorus".to_string(),
        };
        changes.push(Change {
            span: repeat.span.clone(),
            replacement,
            line: repeat.line,
            description: format!(
                "the chorus is the same as the one on line {}: \\insertchorus",
                latest.line
            ),
            removed_lines: text[repeat.span.clone()].matches('\n').count(),
        });
    }
    changes
}

/// Whether `repeat` prints exactly what `latest` does.
fn same_chorus(text: &str, latest: &PartSpan, repeat: &PartSpan) -> bool {
    let rest = |range: &Range<usize>| text[range.clone()].trim().to_string();
    latest.number == repeat.number
        && rest(&latest.opening_rest) == rest(&repeat.opening_rest)
        // Whatever stands behind `\endchorus` (`\rep{2}`) is not part of
        // what `\insertchorus` would print.
        && rest(&repeat.closing_rest).is_empty()
        && !uses_chord_memory(&text[latest.body.clone()])
        && !uses_chord_memory(&text[repeat.body.clone()])
        && lines(&text[latest.body.clone()]) == lines(&text[repeat.body.clone()])
}

/// A part's lines with the spaces inside each one evened out. A line break
/// and a blank line mean something to TeX, so they are kept as they are.
fn lines(body: &str) -> Vec<String> {
    body.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

/// Whether a part replays or stores chords: `^`, `\memorize`, `\usechords`.
fn uses_chord_memory(body: &str) -> bool {
    body.lines().map(code_of).any(|code| {
        code.contains('^') || code.contains("\\memorize") || code.contains("\\usechords")
    })
}

/// A chorus written out by hand as its own reminder (the first line, with
/// its chords off and the rest commented out) becomes `\insertchorus*`,
/// which prints exactly that: the first line of the latest chorus, chords
/// off, with `...`. CLAUDE.md: a chorus that comes back is never retyped and
/// never commented out.
///
/// Only when nothing printed changes:
/// - the reminder has one line sung, ending in `...` (which the reminder
///   adds; a line typed without it would gain it), and shows no chords;
/// - nothing in it depends on the paper (`\iflargepaper`): the reminder
///   macro has one line for all;
/// - its words are the first line's of the chorus that is the latest at that
///   point, which may be an earlier hand-written reminder, since every
///   `\beginchorus` becomes the latest; one with words of its own stays;
/// - it has the latest chorus's number;
/// - no full `\insertchorus` reprints it before the next chorus: that would
///   go back to the chorus before;
/// - no chorus is conditional.
fn hand_written_reminders(text: &str, scan: &Scan) -> Vec<Change> {
    use super::fit::{chord_names, has_word, lyric_words};
    let choruses: Vec<&PartSpan> = scan.parts.iter().filter(|p| p.kind == "chorus").collect();
    if choruses.iter().any(|p| p.conditional) || scan.chorus_repeats.iter().any(|(_, c)| *c) {
        return Vec::new();
    }
    let sung = |part: &PartSpan| -> Vec<String> {
        text[part.body.clone()]
            .lines()
            .map(|l| code_of(l).trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };

    let mut changes = Vec::new();
    let mut latest: Option<&PartSpan> = None;
    for (k, &part) in choruses.iter().enumerate() {
        let lines = sung(part);
        let chords_off = has_word(code_of(&text[part.opening_rest.clone()]), "chordsoff");
        let next_chorus = choruses.get(k + 1).map_or(text.len(), |p| p.span.start);
        let reprinted = scan.chorus_repeats.iter().any(|(span, _)| {
            span.start >= part.span.end
                && span.start < next_chorus
                && !text[span.clone()].starts_with("\\insertchorus*")
        });
        let matches_latest = latest.is_some_and(|latest| {
            let first = sung(latest).into_iter().next().unwrap_or_default();
            latest.number == part.number
                && lines.len() == 1
                && ends_in_dots(&lines[0])
                && !lines[0].contains("\\if")
                && (chords_off || chord_names(&lines[0]).is_empty())
                && !lyric_words(&first).is_empty()
                && lyric_words(&first) == lyric_words(&lines[0])
                && !reprinted
                && rest_is_empty(text, part)
        });
        if !matches_latest {
            latest = Some(part);
            continue;
        }
        let latest = latest.unwrap();
        changes.push(Change {
            span: part.span.clone(),
            replacement: match part.number {
                Some(n) => format!("\\insertchorus*[{n}]"),
                None => "\\insertchorus*".to_string(),
            },
            line: part.line,
            description: format!(
                "the chorus is a reminder of the one on line {}, typed out: \\insertchorus*",
                latest.line
            ),
            removed_lines: text[part.span.clone()].matches('\n').count(),
        });
    }
    changes
}

/// Whether a line's words end in dots: the chords and the `\role` label
/// after them aside.
fn ends_in_dots(line: &str) -> bool {
    let mut words = line;
    if let Some(at) = words.find("\\role") {
        words = &words[..at];
    }
    let mut words = words.trim_end();
    while words.ends_with(']') {
        match words.rfind("\\[") {
            Some(at) => words = words[..at].trim_end(),
            None => break,
        }
    }
    words.ends_with("...") || words.ends_with('…')
}

/// Whether the opening line holds nothing but chord switches: anything else
/// there would be lost with the part.
fn rest_is_empty(text: &str, part: &PartSpan) -> bool {
    let opening = code_of(&text[part.opening_rest.clone()])
        .replace("\\chordsoffonsmallpage", "")
        .replace("\\chordsoff", "");
    opening.trim().is_empty()
}

/// A chord the source drew a letter or two inside its syllable moves to
/// where the syllable starts: `hl\[E7]ubokou` becomes `\[E7]hlubokou`,
/// `D\[E]o` becomes `\[E]Do`. Sites draw a chord over the vowel it is sung
/// on, and leave the consonants that open the syllable in front of it.
///
/// Syllables are found with the TeX hyphenation patterns of the song's
/// `lang`, the ones LaTeX itself uses. The patterns know where a word *may*
/// break, which is not every syllable: English ones find none in "away" or
/// "melodies". So a chord only moves back across consonants. Where the
/// patterns found nothing, the way back to the word's start runs over a
/// vowel, and the chord stays where the source put it. A song with no
/// `lang`, or one with no patterns here, is left as it is.
fn chords_to_syllable_starts(text: &str, scan: &Scan) -> Vec<Change> {
    let Some(hyphenator) = scan
        .lang
        .as_deref()
        .and_then(syllables::Hyphenator::for_lang)
    else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    for part in &scan.parts {
        // An instrumental holds chords in bars, not over syllables.
        if part.kind.starts_with("instrumental") {
            continue;
        }
        let mut line_start = part.body.start;
        for (index, raw) in text[part.body.clone()].split_inclusive('\n').enumerate() {
            let code = code_of(raw.trim_end_matches(['\n', '\r']));
            for (span, replacement) in syllables::moves(code, &hyphenator) {
                changes.push(Change {
                    span: line_start + span.start..line_start + span.end,
                    replacement: replacement.clone(),
                    line: part.line + 1 + index,
                    description: format!(
                        "chord moved to the start of its syllable: {}",
                        replacement.trim()
                    ),
                    removed_lines: 0,
                });
            }
            line_start += raw.len();
        }
    }
    changes
}

mod syllables {
    use hyphenation::{Hyphenator as _, Language, Load, Standard};
    use std::ops::Range;

    pub struct Hyphenator(Standard);

    impl Hyphenator {
        /// The patterns for a song's `lang`, if there are any.
        pub fn for_lang(lang: &str) -> Option<Hyphenator> {
            let language = match lang {
                "czech" => Language::Czech,
                "slovak" => Language::Slovak,
                "english" => Language::EnglishUS,
                _ => return None,
            };
            let mut patterns = Standard::from_embedded(language).ok()?;
            // A chord may fall on the first or last letter's syllable, which
            // TeX's minima are there to keep a hyphen away from.
            patterns.minima = (1, 1);
            Some(Hyphenator(patterns))
        }

        /// The letter indices a syllable starts at, 0 included.
        fn starts(&self, letters: &[char]) -> Vec<usize> {
            let word: String = letters.iter().collect::<String>().to_lowercase();
            // Breaks are byte offsets into the lowercased word; lowercasing a
            // letter can change its length, so they are mapped back by letter.
            let lowered: Vec<usize> = word.char_indices().map(|(at, _)| at).collect();
            if lowered.len() != letters.len() {
                return vec![0];
            }
            let mut starts = vec![0];
            for at in self.0.hyphenate(&word).breaks {
                if let Some(index) = lowered.iter().position(|&b| b == at) {
                    starts.push(index);
                }
            }
            starts
        }
    }

    fn is_vowel(c: char) -> bool {
        "aeiouyáéěíóúůýäëïöüàèìòùâêîôûåæøœ".contains(c.to_lowercase().next().unwrap_or(c))
    }

    /// What a line is read into: a letter of the lyrics, a chord, or
    /// anything else (spaces, punctuation, a macro), which ends a word.
    enum Piece {
        Letter(char, Range<usize>),
        Chord(Range<usize>),
        Other,
    }

    fn pieces(code: &str) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut chars = code.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            if c == '\\' {
                match chars.peek() {
                    Some(&(_, '[')) => {
                        // A chord runs to its `]`; one never closed ends the
                        // line's reading.
                        let Some(close) = code[at..].find(']') else {
                            pieces.push(Piece::Other);
                            break;
                        };
                        let end = at + close + 1;
                        while chars.peek().is_some_and(|&(i, _)| i < end) {
                            chars.next();
                        }
                        pieces.push(Piece::Chord(at..end));
                    }
                    Some(&(_, next)) if next.is_alphabetic() => {
                        // A macro's name is no word of the lyrics.
                        while chars.peek().is_some_and(|&(_, n)| n.is_alphabetic()) {
                            chars.next();
                        }
                        pieces.push(Piece::Other);
                    }
                    _ => {
                        chars.next();
                        pieces.push(Piece::Other);
                    }
                }
            } else if c.is_alphabetic() {
                pieces.push(Piece::Letter(c, at..at + c.len_utf8()));
            } else {
                pieces.push(Piece::Other);
            }
        }
        pieces
    }

    /// The replacements that move a line's chords to their syllables' starts:
    /// each covers the letters the chord moves back over and the chord, and
    /// puts the chord in front of them.
    pub fn moves(code: &str, hyphenator: &Hyphenator) -> Vec<(Range<usize>, String)> {
        let pieces = pieces(code);
        let mut moves = Vec::new();
        let mut i = 0;
        while i < pieces.len() {
            // A word: letters, with chords drawn between them.
            if !matches!(pieces[i], Piece::Letter(..)) {
                i += 1;
                continue;
            }
            let mut letters: Vec<(char, Range<usize>)> = Vec::new();
            // Chords inside the word, with the letter index they stand before.
            let mut chords: Vec<(usize, Range<usize>)> = Vec::new();
            let mut j = i;
            while j < pieces.len() {
                match &pieces[j] {
                    Piece::Letter(c, span) => letters.push((*c, span.clone())),
                    Piece::Chord(span) if matches!(pieces.get(j + 1), Some(Piece::Letter(..))) => {
                        chords.push((letters.len(), span.clone()))
                    }
                    _ => break,
                }
                j += 1;
            }
            i = j;

            let word: Vec<char> = letters.iter().map(|(c, _)| *c).collect();
            // A hum ("mmmm") has no syllables; a chord in it is where it is
            // meant to be.
            if !word.iter().copied().any(is_vowel) {
                continue;
            }
            let starts = hyphenator.starts(&word);
            for (k, (index, chord)) in chords.iter().enumerate() {
                let start = starts
                    .iter()
                    .copied()
                    .filter(|&s| s <= *index)
                    .max()
                    .unwrap_or(0);
                if start == *index || word[start..*index].iter().copied().any(is_vowel) {
                    continue;
                }
                // Another chord in the way stays in the way, and so does
                // one standing right in front of the word: the chord would
                // land on it.
                let from = letters[start].1.start;
                if (k > 0 && chords[k - 1].0 >= start) || code[..from].ends_with(']') {
                    continue;
                }
                let mut replacement = code[chord.clone()].to_string();
                replacement.push_str(&code[from..chord.start]);
                moves.push((from..chord.end, replacement));
            }
        }
        moves
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chorus(head: &str, body: &str, tail: &str) -> String {
        format!("\\beginchorus{head}\n{body}\\endchorus{tail}\n\n")
    }

    fn song(parts: &[String]) -> String {
        format!(
            "\\beginsong{{T}}[\n    lang={{english}},\n]\n\n{}\\endsong\n",
            parts.concat()
        )
    }

    const VERSE: &str = "\\beginverse\n    \\[C]A verse\n\\endverse\n\n";
    const BODY: &str = "    \\[C]Hello \\[G]there,\n    \\[F]hello\n";

    #[test]
    fn an_identical_chorus_becomes_insertchorus() {
        let text = song(&[chorus("", BODY, ""), VERSE.into(), chorus("", BODY, "")]);
        let (tidied, changes) = tidy(&text);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            tidied,
            song(&[
                chorus("", BODY, ""),
                VERSE.into(),
                "\\insertchorus\n\n".into()
            ])
        );
    }

    #[test]
    fn spaces_inside_a_line_do_not_matter_but_lines_do() {
        let spaced = "  \\[C]Hello   \\[G]there,\n        \\[F]hello\n";
        let text = song(&[chorus("", BODY, ""), chorus("", spaced, "")]);
        assert_eq!(tidy(&text).1.len(), 1);

        let joined = "    \\[C]Hello \\[G]there, \\[F]hello\n";
        let text = song(&[chorus("", BODY, ""), chorus("", joined, "")]);
        assert!(tidy(&text).1.is_empty());
    }

    #[test]
    fn a_chorus_with_other_words_or_chords_stays() {
        let words = "    \\[C]Hello \\[G]there,\n    \\[F]goodbye\n";
        let chords = "    \\[C]Hello \\[Am]there,\n    \\[F]hello\n";
        for other in [words, chords] {
            let text = song(&[chorus("", BODY, ""), chorus("", other, "")]);
            assert!(tidy(&text).1.is_empty(), "{other}");
        }
    }

    #[test]
    fn only_the_latest_chorus_counts() {
        let other = "    \\[Am]Another\n";
        let text = song(&[
            chorus("", BODY, ""),
            chorus("", other, ""),
            chorus("", BODY, ""),
        ]);
        assert!(tidy(&text).1.is_empty());
    }

    #[test]
    fn numbers_and_switches_must_match() {
        let numbered = song(&[chorus("[1]", BODY, ""), chorus("[1]", BODY, "")]);
        let (tidied, _) = tidy(&numbered);
        assert!(tidied.contains("\\insertchorus[1]\n"), "{tidied}");

        for (first, second) in [("[1]", ""), ("", "[2]"), ("", "\\chordsoff")] {
            let text = song(&[chorus(first, BODY, ""), chorus(second, BODY, "")]);
            assert!(tidy(&text).1.is_empty(), "{first} {second}");
        }
        let both = song(&[
            chorus("\\chordsoff", BODY, ""),
            chorus("\\chordsoff", BODY, ""),
        ]);
        assert_eq!(tidy(&both).1.len(), 1);
    }

    #[test]
    fn a_repeat_mark_behind_the_chorus_keeps_it() {
        let text = song(&[chorus("", BODY, ""), chorus("", BODY, " \\rep{2}")]);
        assert!(tidy(&text).1.is_empty());
    }

    #[test]
    fn conditional_choruses_and_chord_memory_leave_the_song_alone() {
        let conditional = song(&[
            "\\iflargepaper\n".into(),
            chorus("", BODY, ""),
            "\\fi\n".into(),
            chorus("", BODY, ""),
            chorus("", BODY, ""),
        ]);
        assert!(tidy(&conditional).1.is_empty());

        let memory = "    ^Hello ^there,\n    ^hello\n";
        let text = song(&[chorus("", memory, ""), chorus("", memory, "")]);
        assert!(tidy(&text).1.is_empty());
    }

    #[test]
    fn a_chorus_placed_right_by_hand_matches_once_the_first_is() {
        let text = song(&[
            chorus("", "    D\\[E]o tváře\n", ""),
            chorus("", "    \\[E]Do tváře\n", ""),
        ])
        .replace("english", "czech");
        let (once, _) = tidy(&text);
        assert_eq!(
            once,
            song(&[
                chorus("", "    \\[E]Do tváře\n", ""),
                "\\insertchorus\n\n".into()
            ])
            .replace("english", "czech")
        );
        assert!(tidy(&once).1.is_empty());
    }

    #[test]
    fn a_move_inside_a_replaced_chorus_goes_with_it() {
        let body = "    D\\[E]o tváře\n";
        let text = song(&[chorus("", body, ""), chorus("", body, "")]).replace("english", "czech");
        let (tidied, changes) = tidy(&text);
        // Both copies move, then the second is replaced.
        assert_eq!(changes.len(), 3);
        assert_eq!(
            tidied,
            song(&[
                chorus("", "    \\[E]Do tváře\n", ""),
                "\\insertchorus\n\n".into()
            ])
            .replace("english", "czech")
        );
    }

    #[test]
    fn tidying_twice_changes_nothing_more() {
        let text = song(&[
            chorus("", BODY, ""),
            VERSE.into(),
            chorus("", BODY, ""),
            chorus("", BODY, ""),
        ]);
        let (once, changes) = tidy(&text);
        assert_eq!(changes.len(), 2);
        let (twice, more) = tidy(&once);
        assert!(more.is_empty());
        assert_eq!(once, twice);
    }

    fn verse_song(lang: &str, line: &str) -> String {
        format!(
            "\\beginsong{{T}}[\n    lang={{{lang}}},\n]\n\n\\beginverse\n    {line}\n\\endverse\n\n\\endsong\n"
        )
    }

    fn tidied_line(lang: &str, line: &str) -> String {
        let (text, _) = tidy(&verse_song(lang, line));
        text.lines().nth(5).unwrap().trim().to_string()
    }

    #[test]
    fn a_chord_moves_back_over_the_consonants_opening_its_syllable() {
        assert_eq!(
            tidied_line(
                "czech",
                "D\\[E]o tváře čas mi hl\\[E7]ubokou brázdu s\\[A]amotou"
            ),
            "\\[E]Do tváře čas mi \\[E7]hlubokou brázdu \\[A]samotou"
        );
        assert_eq!(
            tidied_line("czech", "nikd\\[E]o ti nepov\\[C#m]í"),
            "ni\\[E]kdo ti nepo\\[C#m]ví"
        );
        assert_eq!(tidied_line("czech", "kap\\[H7]itán"), "ka\\[H7]pitán");
    }

    #[test]
    fn a_chord_already_on_its_syllable_stays() {
        for line in [
            "nedo\\[Fmaj7]jdou",
            "\\[C]Vydal z~hrdla",
            "conver\\[G#m]sation",
            "slova\\[G]",
        ] {
            let lang = if line.contains("conver") {
                "english"
            } else {
                "czech"
            };
            assert_eq!(tidied_line(lang, line), line);
        }
    }

    #[test]
    fn where_the_patterns_know_no_syllable_nothing_moves() {
        // English patterns find no break in "melodies" or "away"; the way back
        // runs over vowels, so the chord stays.
        assert_eq!(
            tidied_line("english", "ancient melo\\[A]dies"),
            "ancient melo\\[A]dies"
        );
        assert_eq!(tidied_line("english", "a\\[C]way"), "a\\[C]way");
        assert_eq!(tidied_line("english", "mmm\\[Hm]m"), "mmm\\[Hm]m");
    }

    #[test]
    fn a_song_without_lang_or_patterns_is_left_alone() {
        let text = verse_song("french", "D\\[E]o");
        assert!(tidy(&text).1.is_empty());
        let text = "\\beginsong{T}[\n]\n\\beginverse\n    D\\[E]o\n\\endverse\n\\endsong\n";
        assert!(tidy(text).1.is_empty());
    }

    #[test]
    fn macros_comments_and_instrumentals_are_not_words() {
        assert_eq!(
            tidied_line("czech", "\\enquote{D\\[E]o} % D\\[E]o"),
            "\\enquote{\\[E]Do} % D\\[E]o"
        );
        let text = "\\beginsong{T}[\n    lang={czech},\n]\n\\begininstrumental\n    | D\\[E]o |\n\\endinstrumental\n\\endsong\n";
        assert!(tidy(text).1.is_empty());
    }

    #[test]
    fn a_chord_in_the_way_keeps_the_next_one_where_it_is() {
        // "hlubokou": a chord before "bo" and one inside it; the second may
        // not jump over the first.
        assert_eq!(
            tidied_line("czech", "hlu\\[C]b\\[D]okou"),
            "hlu\\[C]b\\[D]okou"
        );
        // Nor onto a chord drawn right in front of the word.
        assert_eq!(
            tidied_line("czech", "jak \\[G]p\\[C]í\\[D]ská"),
            "jak \\[G]p\\[C]í\\[D]ská"
        );
    }

    #[test]
    fn a_reminder_typed_by_hand_becomes_insertchorus_star() {
        let full = "    \\[G]Carry out your \\[Em]task, \\[C]spheres \\role{Bird}\n    \\[F]Live out our dreams\n";
        let typed = "    \\[G]Carry out your \\[Em]task, \\[C]spheres... \\role{Bird}\n    %\\[F]Live out our dreams\n";
        let text = song(&[
            chorus("", full, ""),
            VERSE.into(),
            chorus("\\chordsoff", typed, ""),
            VERSE.into(),
            chorus("\\chordsoff", typed, " \\rep{2}"),
        ]);
        let (tidied, changes) = tidy(&text);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(
            tidied,
            song(&[
                chorus("", full, ""),
                VERSE.into(),
                "\\insertchorus*\n\n".into(),
                VERSE.into(),
                "\\insertchorus* \\rep{2}\n\n".into(),
            ])
        );
        assert!(tidy(&tidied).1.is_empty());
    }

    #[test]
    fn a_reminder_in_words_of_its_own_stays_and_is_the_latest() {
        let full = "    Nechám \\[Dm]déšť, ať padá si dál\n    neuhasí \\[C]žár\n";
        let own = "    \\[Dm]Déšť ať padá si dál...\n    %neuhasí \\[C]žár\n";
        let first = "    Nechám \\[Dm]déšť ať padá si dál...\n    %neuhasí \\[C]žár\n";
        // The second reminder repeats the first line of the full chorus, but
        // the latest chorus by then is the first reminder: it stays too.
        let text = song(&[
            chorus("", full, ""),
            chorus("\\chordsoff", own, ""),
            chorus("\\chordsoff", first, ""),
        ])
        .replace("english", "czech");
        let (_, changes) = tidy(&text);
        assert!(
            changes.iter().all(|c| !c.description.contains("reminder")),
            "{changes:?}"
        );
    }

    #[test]
    fn a_reminder_with_chords_or_reprinted_in_full_stays() {
        let full = "    \\[C]Fear of the \\[Dm]dark\n    \\[F]I have a phobia\n";
        let chorded = "    \\[C]Fear of the \\[Dm]dark...\n    %\\[F]I have a phobia\n";
        let text = song(&[chorus("", full, ""), chorus("", chorded, "")]);
        assert!(tidy(&text).1.is_empty());

        let typed = "    \\[C]Fear of the \\[Dm]dark...\n";
        let text = song(&[
            chorus("", full, ""),
            chorus("\\chordsoff", typed, ""),
            "\\insertchorus\n\n".into(),
        ]);
        assert!(
            tidy(&text)
                .1
                .iter()
                .all(|c| !c.description.contains("reminder"))
        );
    }

    #[test]
    fn lines_are_reported_as_the_song_was_given() {
        let full = "    \\[G]Carry on \\[C]spheres\n    \\[F]Live out\n";
        let typed = "    \\[G]Carry on \\[C]spheres...\n    %\\[F]Live out\n";
        let text = song(&[
            chorus("", full, ""),
            chorus("\\chordsoff", typed, ""),
            "\\beginverse\n    D\\[E]o\n\\endverse\n\n".into(),
        ])
        .replace("english", "czech");
        let (_, changes) = tidy(&text);
        let moved = changes
            .iter()
            .find(|c| c.description.contains("syllable"))
            .unwrap();
        let line = text.lines().position(|l| l.contains("D\\[E]o")).unwrap() + 1;
        assert_eq!(moved.line, line);
    }

    #[test]
    fn a_reminder_without_dots_or_depending_on_the_paper_stays() {
        let full = "    \\[C]Hallelujah, \\[Am]hallelujah\n    \\[F]Hallelujah\n";
        let no_dots = "    Hallelujah, hallelujah\n";
        let text = song(&[chorus("", full, ""), chorus("\\chordsoff", no_dots, "")]);
        assert!(tidy(&text).1.is_empty());

        let paper = "    Hallelujah\\iflargepaper , hallelujah\\fi...\n";
        let text = song(&[chorus("", full, ""), chorus("\\chordsoff", paper, "")]);
        assert!(tidy(&text).1.is_empty());
    }

    #[test]
    fn dots_count_before_trailing_chords_and_a_role() {
        assert!(ends_in_dots("never cared for what they \\[C]do... \\[A]"));
        assert!(ends_in_dots("spheres... \\role{Bird}{Ω}"));
        assert!(ends_in_dots("Mornie utúlië…"));
        assert!(!ends_in_dots("Hallelujah, hallelujah \\[C]"));
    }
}
