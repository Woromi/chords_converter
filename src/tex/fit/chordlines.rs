//! Chords that change lines from one part to the next, for `check`.
//!
//! Parts of a kind that play the same chords usually break their lines at
//! the same chords too, but sites often put a chord sung between two lines
//! at the end of the first line in one verse and at the start of the next
//! in another. The verses then read differently for no reason, and joining
//! lines (`join`) or hiding chords (`shrink`) works from parts that do not
//! line up. So when parts of a kind have the same chords in the same order
//! and as many lines, but split them across the lines differently, the
//! split most of them share (the first part's on a tie) is offered to the
//! others. It is not a rule, since a verse may really be sung differently
//! (the user), so each is a finding for a person, and `% nocheck` marks one
//! right. Only a chord that moves cleanly is moved: one after the last word
//! of its line (to the start of the next line) or one before the first
//! word of its line (to the end of the previous one). Anything else is left
//! alone, unreported.

use super::{
    Edit, Names, PartInfo, Proposal, Source, Stage, chord_names, code_of, is_nocheck_mark, line_at,
    lyric_words, source_of,
};
use std::ops::Range;

/// A chord in a line: its name, its bytes in the song, and whether no
/// lyric comes before it (`leading`) or after it (`trailing`) in its line.
struct Placed {
    name: String,
    span: Range<usize>,
    leading: bool,
    trailing: bool,
}

/// A sung line: its code's bytes (indentation and comment left out) and
/// its chords.
struct Row {
    range: Range<usize>,
    chords: Vec<Placed>,
}

fn rows_of(text: &str, info: &PartInfo) -> Vec<Row> {
    let body = info.part.body.clone();
    let mut rows = Vec::new();
    let mut at = body.start;
    for raw in text[body].split_inclusive('\n') {
        let line_start = at;
        at += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        if is_nocheck_mark(line) {
            continue;
        }
        let code = code_of(line);
        // A chord switch on a line of its own prints nothing.
        if code.trim().is_empty() || super::inside::is_switch(code) {
            continue;
        }
        let indent = code.len() - code.trim_start().len();
        let code = code.trim_end();
        let mut chords = Vec::new();
        let mut from = 0;
        while let Some(found) = code[from..].find("\\[") {
            let start = from + found;
            let Some(close) = code[start..].find(']') else {
                break;
            };
            let end = start + close + 1;
            from = end;
            // The same chords as `chord_names` counts.
            if let [name] = chord_names(&code[start..end]).as_slice() {
                chords.push(Placed {
                    name: name.clone(),
                    span: line_start + start..line_start + end,
                    leading: lyric_words(&code[..start]).is_empty(),
                    trailing: lyric_words(&code[end..]).is_empty(),
                });
            }
        }
        rows.push(Row {
            range: line_start + indent..line_start + code.len(),
            chords,
        });
    }
    rows
}

fn counts(rows: &[Row]) -> Vec<usize> {
    rows.iter().map(|r| r.chords.len()).collect()
}

fn names_of(rows: &[Row]) -> Vec<&str> {
    rows.iter()
        .flat_map(|r| r.chords.iter().map(|c| c.name.as_str()))
        .collect()
}

pub(super) fn chords_across_lines(infos: &[PartInfo], names: &Names) -> Vec<Proposal> {
    let text = names.text;
    let mut kinds: Vec<&str> = infos.iter().map(|i| i.part.kind.as_str()).collect();
    kinds.sort();
    kinds.dedup();
    let mut proposals = Vec::new();
    for kind in kinds {
        // An instrumental's rows are bars, not lines of a phrase.
        if kind.starts_with("instrumental") {
            continue;
        }
        // Parts whose chords can be read (none replayed with `^` or
        // switched mid-part), on every paper alike.
        let parts: Vec<(usize, Vec<Row>)> = infos
            .iter()
            .enumerate()
            .filter(|(_, i)| i.part.kind == kind && !i.part.conditional && i.chords.is_some())
            .map(|(n, i)| (n, rows_of(text, i)))
            .filter(|(_, rows)| rows.iter().any(|r| !r.chords.is_empty()))
            .collect();
        let mut grouped = vec![false; parts.len()];
        for a in 0..parts.len() {
            if grouped[a] {
                continue;
            }
            let alike = |b: usize| {
                parts[b].1.len() == parts[a].1.len()
                    && names_of(&parts[b].1) == names_of(&parts[a].1)
            };
            let group: Vec<usize> = (a..parts.len())
                .filter(|&b| !grouped[b] && alike(b))
                .collect();
            for &b in &group {
                grouped[b] = true;
            }
            // The split most share, the earliest of those on a tie.
            let Some(&reference) = group.iter().max_by_key(|&&g| {
                let shared = group
                    .iter()
                    .filter(|&&h| counts(&parts[h].1) == counts(&parts[g].1))
                    .count();
                (shared, std::cmp::Reverse(g))
            }) else {
                continue;
            };
            let wanted = counts(&parts[reference].1);
            for &g in &group {
                if counts(&parts[g].1) == wanted {
                    continue;
                }
                let (part, rows) = &parts[g];
                let (other, theirs) = &parts[reference];
                let source = source_of(infos[*other].part);
                if let Some(p) = moved(text, names, *part, rows, *other, theirs, source) {
                    proposals.push(p);
                }
            }
        }
    }
    proposals
}

/// `rows`' chords moved across line ends to the split of `theirs`, if every
/// chord that moves sits after the last word of its line or before the
/// first, and `theirs` has it the other way round: what the other parts
/// sing between the lines, this one does too, only written on the other
/// line.
fn moved(
    text: &str,
    names: &Names,
    part: usize,
    rows: &[Row],
    reference: usize,
    theirs: &[Row],
    source: Source,
) -> Option<Proposal> {
    let have = counts(rows);
    let wanted = counts(theirs);
    // How many more chords the part has than wanted up to the end of each
    // line: positive, its last chords go down to the next line; negative,
    // the next line's first chords come up.
    let mut shift = Vec::new();
    let (mut a, mut b) = (0i64, 0i64);
    for k in 0..rows.len() - 1 {
        a += have[k] as i64;
        b += wanted[k] as i64;
        shift.push(a - b);
    }
    // A chord moves across one line end at most.
    for k in 0..rows.len() {
        let up = if k > 0 { (-shift[k - 1]).max(0) } else { 0 };
        let down = shift.get(k).map_or(0, |&s| s.max(0));
        if (up + down) as usize > have[k] {
            return None;
        }
    }

    let mut edits = Vec::new();
    let mut said = Vec::new();
    let mut first_line = None;
    let spaces = |c: char| c == ' ' || c == '\t';
    let list = |chords: &[Placed]| {
        let names: Vec<&str> = chords.iter().map(|c| c.name.as_str()).collect();
        match names.split_last() {
            Some((last, [])) => last.to_string(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        }
    };
    for (k, &s) in shift.iter().enumerate() {
        let (this, next) = (&rows[k], &rows[k + 1]);
        let from_line = line_at(text, this.range.start);
        let to_line = line_at(text, next.range.start);
        if s > 0 {
            let chords = &this.chords[this.chords.len() - s as usize..];
            // In theirs, the same chords start the next line.
            let there = theirs[k + 1].chords.get(..s as usize)?;
            if !chords.iter().all(|c| c.trailing) || !there.iter().all(|c| c.leading) {
                return None;
            }
            // From the end of the last word to the end of the line.
            let start = this.range.start
                + text[this.range.start..chords[0].span.start]
                    .trim_end_matches(spaces)
                    .len();
            let moving: Vec<&str> = chords.iter().map(|c| &text[c.span.clone()]).collect();
            let mut moving = moving.join(" ");
            // Before a chord of its own, a space keeps the two apart.
            if text[next.range.clone()].starts_with("\\[") {
                moving.push(' ');
            }
            edits.push(Edit {
                span: start..this.range.end,
                replacement: String::new(),
            });
            edits.push(Edit {
                span: next.range.start..next.range.start,
                replacement: moving,
            });
            first_line.get_or_insert(from_line);
            said.push(format!(
                "{} from the end of line {from_line} to the start of line {to_line}",
                list(chords)
            ));
        } else if s < 0 {
            let chords = &next.chords[..(-s) as usize];
            // In theirs, the same chords end this line.
            let at = theirs[k].chords.len().checked_sub((-s) as usize)?;
            let there = &theirs[k].chords[at..];
            if !chords.iter().all(|c| c.leading) || !there.iter().all(|c| c.trailing) {
                return None;
            }
            // Nothing but the chords and spaces between them goes up.
            let mut end = next.range.start;
            for c in chords {
                if !text[end..c.span.start].trim_matches(spaces).is_empty() {
                    return None;
                }
                end = c.span.end;
            }
            end += text[end..next.range.end].len()
                - text[end..next.range.end].trim_start_matches(spaces).len();
            // A line of these chords alone would be left empty.
            if end >= next.range.end {
                return None;
            }
            let moving: Vec<&str> = chords.iter().map(|c| &text[c.span.clone()]).collect();
            edits.push(Edit {
                span: this.range.end..this.range.end,
                replacement: format!(" {}", moving.join(" ")),
            });
            edits.push(Edit {
                span: next.range.start..end,
                replacement: String::new(),
            });
            first_line.get_or_insert(to_line);
            said.push(format!(
                "{} from the start of line {to_line} to the end of line {from_line}",
                list(chords)
            ));
        }
    }
    let line = first_line?;
    Some(Proposal {
        id: format!("chordlines@{line}"),
        kind: "chordlines",
        stage: Stage::AllPapers,
        line,
        description: format!(
            "{} splits its chords across the lines unlike {}: {}",
            names.parts[part],
            names.parts[reference],
            said.join("; ")
        ),
        gain_lines: 0,
        score: 0,
        edits,
        sources: vec![source],
        effects: Vec::new(),
        joins: None,
    })
}
