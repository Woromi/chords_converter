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
}

/// Tidies a song: every rule, every change applied. Tidying what this
/// returns again changes nothing.
pub fn tidy(text: &str) -> (String, Vec<Change>) {
    let mut changes = repeated_choruses(text, &scan(text));
    changes.sort_by_key(|c| c.span.start);
    (apply(text, &changes), changes)
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
}
