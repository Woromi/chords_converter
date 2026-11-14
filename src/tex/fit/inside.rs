//! Chords hidden inside a part whose lines play the chords of its first
//! lines again (the user): a verse of eight lines going D A Hm G two lines
//! at a time shows them over its first two lines and hides them after,
//! with `\chordsoff` or `\chordsoffonsmallpage` on a line of its own. The
//! reader looks back a line or two, inside the part, so it reads better
//! than a part without chords that sends the reader to another part.
//!
//! The lines are compared one by one: line `k` of each repeat with line
//! `k` of the first, which is where the reader looks. A line may leave out
//! its first chord when that chord is still sounding from the line before
//! (the second half of a verse starting `Your smile is like a \[C]breath`,
//! where the first started `Your \[Am]beauty is be\[C]yond`, with Am rung
//! out at the end of the line before): that is the same chord, unwritten.
//! The first lines that repeat to the end of the part are taken, the
//! fewest, since that hides the most; checked against the songbook, where
//! the user had placed the switch by hand, it lands on the same line every
//! time it finds one.

use super::{
    Chords, Edit, Names, PartInfo, Proposal, Stage, chord_names, code_of, line_at, source_of,
    switchable,
};

/// Readability left by chords hidden inside a part: every word stays, and
/// the chords are a line or two back, in the same part. Better than a whole
/// part without chords (`CHORDS_OFF`), not as good as a reminder.
const CHORDS_INSIDE: u8 = 75;

/// A sung line of a part: where it starts, its indentation, and its chords.
pub(super) struct Row {
    start: usize,
    indent: usize,
    pub(super) chords: Vec<String>,
}

/// The sung lines of a part's body: lines with code, a chord switch on its
/// own aside, and where such a switch is, as the number of rows before it.
pub(super) fn rows(text: &str, body: std::ops::Range<usize>) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut switches = Vec::new();
    let mut at = body.start;
    for raw in text[body].split_inclusive('\n') {
        let start = at;
        at += raw.len();
        let code = code_of(raw.trim_end_matches(['\n', '\r']));
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_switch(trimmed) {
            switches.push(rows.len());
            continue;
        }
        rows.push(Row {
            start,
            indent: code.len() - code.trim_start().len(),
            chords: chord_names(trimmed),
        });
    }
    (rows, switches)
}

/// Whether a line's code is nothing but a switch that hides chords.
pub(super) fn is_switch(code: &str) -> bool {
    matches!(code.trim(), "\\chordsoff" | "\\chordsoffonsmallpage")
}

/// Whether the rows from `p` on play the chords of the first `p` again,
/// line by line, and so can be read there.
pub(super) fn repeats(rows: &[Row], p: usize) -> bool {
    if p == 0 || p >= rows.len() || !rows.len().is_multiple_of(p) {
        return false;
    }
    // The chord sounding as each row starts: the last one before it.
    let mut sounding = Vec::with_capacity(rows.len());
    let mut last: Option<&str> = None;
    for row in rows {
        sounding.push(last);
        if let Some(c) = row.chords.last() {
            last = Some(c);
        }
    }
    let same = |k: usize| {
        let (first, again) = (&rows[k % p].chords, &rows[k].chords);
        first == again
            || (again.as_slice() == first.get(1..).unwrap_or_default()
                && sounding[k] == first.first().map(String::as_str))
            || (first.as_slice() == again.get(1..).unwrap_or_default()
                && sounding[k % p] == again.first().map(String::as_str))
    };
    rows[..p].iter().any(|r| !r.chords.is_empty()) && (p..rows.len()).all(same)
}

/// The fewest first rows the rest of the part repeats, if any repeat has
/// chords to hide.
fn period(rows: &[Row]) -> Option<usize> {
    (1..=rows.len() / 2)
        .find(|&p| repeats(rows, p))
        .filter(|&p| rows[p..].iter().any(|r| !r.chords.is_empty()))
}

pub(super) fn chords_inside(names: &Names, infos: &[PartInfo], stage: Stage) -> Vec<Proposal> {
    let text = names.text;
    let (word, kind) = match stage {
        Stage::AllPapers => ("\\chordsoff", "chordsoffinside"),
        Stage::SmallPaper => ("\\chordsoffonsmallpage", "chordsoffonsmallpageinside"),
    };
    let mut proposals = Vec::new();
    for (n, info) in infos.iter().enumerate() {
        // Instrumentals are bars, and a part replaying (`^`), switching
        // already or hiding its chords from the start has none to hide.
        if !switchable(info)
            || info.chords.is_none()
            || info.mid_switch
            || info.switch != Chords::Shown
            || text[info.part.body.clone()].lines().any(|l| {
                ["\\if", "\\else", "\\fi"]
                    .iter()
                    .any(|w| code_of(l).contains(w))
            })
        {
            continue;
        }
        let (rows, _) = rows(text, info.part.body.clone());
        let Some(p) = period(&rows) else {
            continue;
        };
        let row = &rows[p];
        let line = line_at(text, row.start);
        let first = line_at(text, rows[0].start);
        let last = line_at(text, rows[rows.len() - 1].start);
        let lines = |a: usize, b: usize| {
            if a == b {
                format!("line {a}")
            } else {
                format!("lines {a}–{b}")
            }
        };
        let shown = lines(first, line_at(text, rows[p - 1].start));
        let hidden = lines(line, last);
        proposals.push(Proposal {
            id: format!("{kind}@{line}"),
            kind,
            stage,
            line,
            description: format!(
                "{hidden} of {} {} the chords of {shown} again: {word} before line {line}",
                names.parts[n],
                if p + 1 == rows.len() { "plays" } else { "play" },
            ),
            gain_lines: rows[p..].iter().filter(|r| !r.chords.is_empty()).count(),
            score: CHORDS_INSIDE,
            edits: vec![Edit {
                span: row.start..row.start,
                replacement: format!("{}{word}\n", &text[row.start..row.start + row.indent]),
            }],
            sources: vec![source_of(info.part)],
            effects: Vec::new(),
            joins: None,
        });
    }
    proposals
}
