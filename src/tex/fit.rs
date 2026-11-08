//! Tier 3: changes to what a song prints, which a person decides on. Each is
//! offered as a proposal: what it does, roughly how many lines it saves, and
//! the bytes it would replace. Nothing here changes a song by itself.
//!
//! Fitting a page has two stages (decided with the user). First the song is
//! made to fit A4 with changes that act on every paper, since a song that
//! overflows A4 overflows A5 too: `\insertchorus*` for a chorus sung again,
//! `\chordsoff` on parts that reprint chords already shown. Once A4 fits,
//! the changes act on small paper only: `\chordsoffonsmallpage`, then
//! `\smalleronsmallpage` as the last resort (CLAUDE.md "Fitting the page").
//!
//! Within a stage, proposals are rated by how readable they leave the song
//! (decided with the user), not by the space they save alone. Best is all
//! the words and all the chords where they are sung. A chorus shortened to
//! its reminder line costs a glance back to the chorus. A part without its
//! chords costs more: the words are read in one place and the chords in
//! another, line after line. Where chords go off matters too: chorded and
//! plain parts should alternate (on, off, on, off rather than on, on, off,
//! off), and the chords stay towards the beginning. A smaller type costs the
//! whole song, so it comes last.

use super::{PartSpan, Scan, code_of, scan};
use std::ops::Range;

/// The paper a change acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Every paper: A4 and A5 alike.
    AllPapers = 1,
    /// Small paper (A5) only.
    SmallPaper = 2,
}

/// One replacement of bytes in the song.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub span: Range<usize>,
    pub replacement: String,
}

/// A change offered to the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// Stable for the same text: `<kind>@<line>`.
    pub id: String,
    pub kind: &'static str,
    pub stage: Stage,
    /// The 1-based line it is about.
    pub line: usize,
    pub description: String,
    /// Printed lines it saves, estimated: a chord row counts as a line.
    pub gain_lines: usize,
    /// How readable the song stays, 0–100, higher is better.
    pub score: u8,
    pub edits: Vec<Edit>,
    /// The part whose chords it switches off, and how, for checking that
    /// the proposals chosen together still keep chords spread out.
    effect: Option<(usize, Chords)>,
}

/// The readability a change leaves, before its place is taken into account.
const REMINDER: u8 = 80;
const CHORDS_OFF: u8 = 60;
const SMALLER_TYPE: u8 = 20;
/// Taken off for each neighbour of the kind that is plain already: two
/// plain parts side by side are what alternating avoids.
const NEXT_TO_PLAIN: u8 = 15;
/// Taken off, at most, for a change on the first part that can take it,
/// less further on, so the words and chords stay towards the beginning.
/// Small enough that the place never changes a grade on its own.
const EARLY: u8 = 5;

impl Proposal {
    /// The rating in words: what a person reads next to the score.
    pub fn grade(&self) -> &'static str {
        match self.score {
            75.. => "good",
            55.. => "fair",
            35.. => "poor",
            _ => "last resort",
        }
    }
}

/// How much earlier is worse: `EARLY` for the first of `n` places a change
/// could go (the first part of a kind never goes plain, so `k` starts at 1
/// there), nothing for the last.
fn early(k: usize, n: usize) -> u8 {
    if n < 2 {
        return 0;
    }
    // Rounded up, so only the last place goes free.
    (EARLY as usize * (n - 1 - k.min(n - 1))).div_ceil(n - 1) as u8
}

/// Whether a part shows its chords, and on which paper.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Chords {
    Shown,
    /// `\chordsoffonsmallpage`: shown on A4, hidden on A5.
    OffOnSmallPage,
    /// `\chordsoff`: hidden everywhere.
    Off,
}

impl Chords {
    fn plain_on(self, stage: Stage) -> bool {
        match stage {
            Stage::AllPapers => self == Chords::Off,
            Stage::SmallPaper => self != Chords::Shown,
        }
    }
}

/// What the generators need to know about one part.
struct PartInfo<'a> {
    part: &'a PartSpan,
    /// The chords of its body, in order; `None` when it replays them (`^`)
    /// or switches chords on and off inside, so they can't be compared.
    chords: Option<Vec<String>>,
    switch: Chords,
    /// Lines it prints, chord rows aside.
    lyric_rows: usize,
    /// Of those, the ones with chords over them.
    chorded_rows: usize,
    /// Its words, chords and macros left out, for comparing lyrics.
    words: String,
}

fn info<'a>(text: &str, part: &'a PartSpan) -> PartInfo<'a> {
    let opening = code_of(&text[part.opening_rest.clone()]);
    let switch = if has_word(opening, "chordsoff") {
        Chords::Off
    } else if has_word(opening, "chordsoffonsmallpage") {
        Chords::OffOnSmallPage
    } else {
        Chords::Shown
    };

    let mut chords = Vec::new();
    let mut replayed = false;
    let mut lyric_rows = 0;
    let mut chorded_rows = 0;
    let mut words = String::new();
    for line in text[part.body.clone()].lines() {
        let code = code_of(line).trim();
        if code.is_empty() {
            continue;
        }
        if has_word(code, "chordsoff")
            || has_word(code, "chordson")
            || has_word(code, "chordsoffonsmallpage")
            || code.contains('^')
        {
            replayed = true;
        }
        lyric_rows += 1;
        let line_chords = chord_names(code);
        if !line_chords.is_empty() {
            chorded_rows += 1;
        }
        chords.extend(line_chords);
        words.push_str(&lyric_words(code));
        words.push('\n');
    }
    PartInfo {
        part,
        chords: (!replayed).then_some(chords),
        switch,
        lyric_rows,
        chorded_rows,
        words,
    }
}

/// Whether `code` holds the control word `\word` (and not one it starts).
pub(super) fn has_word(code: &str, word: &str) -> bool {
    let needle = format!("\\{word}");
    code.match_indices(&needle).any(|(at, _)| {
        !code[at + needle.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    })
}

/// The names of the chords in a line, `\[]` slots and `\[\%]` aside.
pub(super) fn chord_names(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("\\[") {
        let after = &rest[at + 2..];
        let Some(close) = after.find(']') else { break };
        let name = after[..close].trim();
        if !name.is_empty() && name != "\\%" {
            names.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    names
}

/// A line's words, lowercased, with chords, macros and punctuation left out.
pub(super) fn lyric_words(code: &str) -> String {
    let mut out = String::new();
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if c == ']' {
                            break;
                        }
                    }
                }
                Some(n) if n.is_alphabetic() => {
                    while chars.peek().is_some_and(|n| n.is_alphabetic()) {
                        chars.next();
                    }
                }
                _ => {
                    chars.next();
                }
            }
            out.push(' ');
        } else if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if matches!(c, '\'' | '’' | '´' | '`') {
            // "I'll" and "Ill" are the same word sung, typed two ways.
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every change the song could take, best first: stage by stage, and within
/// a stage by the readability it leaves (`score`), then by the lines it
/// saves, then the later first. A change that saves no line is not offered.
pub fn proposals(text: &str) -> Vec<Proposal> {
    let scan = scan(text);
    let infos: Vec<PartInfo> = scan.parts.iter().map(|p| info(text, p)).collect();

    let mut proposals = Vec::new();
    proposals.extend(chords_off(text, &infos, Stage::AllPapers));
    proposals.extend(chorus_reminders(text, &scan, &infos));
    proposals.extend(chords_off(text, &infos, Stage::SmallPaper));
    proposals.extend(smaller_type(text, &scan, &infos));

    proposals.retain(|p| p.gain_lines > 0);
    let rank = |p: &Proposal| {
        (
            p.stage,
            std::cmp::Reverse(p.score),
            std::cmp::Reverse(p.gain_lines),
            // The later of two otherwise equal: chords and words stay
            // towards the beginning.
            std::cmp::Reverse(p.line),
        )
    };
    proposals.sort_by_key(rank);
    proposals
}

/// The parts that can take a chord switch: sung parts, not instrumentals.
fn switchable(info: &PartInfo) -> bool {
    !info.part.kind.starts_with("instrumental") && !info.part.conditional
}

/// The chords of a part that repeats an earlier part of its kind, chord for
/// chord, can be hidden: on every paper (`\chordsoff`) or on small paper
/// (`\chordsoffonsmallpage`). The earlier part has to show its chords on the
/// paper in question, and the parts of the kind have to stay spread out: the
/// first shows its chords, and every one without has a neighbour with.
fn chords_off(text: &str, infos: &[PartInfo], stage: Stage) -> Vec<Proposal> {
    let (word, kind) = match stage {
        Stage::AllPapers => ("chordsoff", "chordsoff"),
        Stage::SmallPaper => ("chordsoffonsmallpage", "chordsoffonsmallpage"),
    };
    let mut proposals = Vec::new();
    for (i, info) in infos.iter().enumerate() {
        let Some(chords) = &info.chords else { continue };
        if chords.is_empty() || !switchable(info) || info.switch.plain_on(stage) {
            continue;
        }
        // The earlier part shows its chords on every paper.
        let shown_before = infos[..i].iter().find(|earlier| {
            earlier.part.kind == info.part.kind
                && earlier.chords.as_ref() == Some(chords)
                && earlier.switch == Chords::Shown
        });
        let Some(earlier) = shown_before else {
            continue;
        };
        let target = match stage {
            Stage::AllPapers => Chords::Off,
            Stage::SmallPaper => Chords::OffOnSmallPage,
        };
        if !spread_out(infos, &[(i, target)]) {
            continue;
        }

        // Its place among the parts of its kind, and the plain ones beside
        // it on small paper: a change for every paper acts there too, so a
        // neighbour plain only on A5 is plain beside it all the same.
        let kin: Vec<usize> = (0..infos.len())
            .filter(|&j| infos[j].part.kind == info.part.kind && switchable(&infos[j]))
            .collect();
        let k = kin.iter().position(|&j| j == i).unwrap();
        let plain = |j: usize| {
            infos[j].chords.as_ref().is_some_and(|c| c.is_empty())
                || infos[j].switch.plain_on(Stage::SmallPaper)
        };
        let plain_beside = [k.checked_sub(1), Some(k + 1)]
            .into_iter()
            .flatten()
            .filter_map(|n| kin.get(n))
            .filter(|&&j| plain(j))
            .count() as u8;
        let score = CHORDS_OFF - NEXT_TO_PLAIN * plain_beside - early(k, kin.len());

        // A part already off on small paper switches to off everywhere by
        // trading the one word for the other.
        let opening = &info.part.opening_rest;
        let edit = match (stage, info.switch) {
            (Stage::AllPapers, Chords::OffOnSmallPage) => {
                let at = opening.start
                    + text[opening.clone()]
                        .find("\\chordsoffonsmallpage")
                        .unwrap();
                Edit {
                    span: at..at + "\\chordsoffonsmallpage".len(),
                    replacement: "\\chordsoff".to_string(),
                }
            }
            _ => Edit {
                span: opening.start..opening.start,
                replacement: format!("\\{word}"),
            },
        };
        proposals.push(Proposal {
            id: format!("{kind}@{}", info.part.line),
            kind,
            stage,
            line: info.part.line,
            description: format!(
                "the {} repeats the chords of line {}: \\{}{}",
                part_name(&info.part.kind),
                earlier.part.line,
                word,
                if plain_beside > 0 {
                    format!(
                        " (next to a {} already without chords)",
                        part_name(&info.part.kind)
                    )
                } else {
                    String::new()
                }
            ),
            gain_lines: info.chorded_rows,
            score,
            edits: vec![edit],
            effect: Some((i, target)),
        });
    }
    proposals
}

/// Whether, with the given parts switched, every kind of part still has its
/// chords spread out on every paper: the first of a kind shows them, and
/// each part without them has a neighbour of its kind with them.
fn spread_out(infos: &[PartInfo], changes: &[(usize, Chords)]) -> bool {
    let changed = |i: usize| {
        changes
            .iter()
            .rev()
            .find(|(at, _)| *at == i)
            .map(|(_, s)| *s)
            .unwrap_or(infos[i].switch)
    };
    let mut kinds: Vec<&str> = infos.iter().map(|i| i.part.kind.as_str()).collect();
    kinds.sort();
    kinds.dedup();
    for paper in [Stage::AllPapers, Stage::SmallPaper] {
        for kind in &kinds {
            let parts: Vec<usize> = (0..infos.len())
                .filter(|&i| infos[i].part.kind == *kind && switchable(&infos[i]))
                .collect();
            // A part written without chords is as plain as one switched off.
            let plain = |switch: &dyn Fn(usize) -> Chords| -> Vec<bool> {
                parts
                    .iter()
                    .map(|&i| {
                        infos[i].chords.as_ref().is_some_and(|c| c.is_empty())
                            || switch(i).plain_on(paper)
                    })
                    .collect()
            };
            // A song may break the rule already; that is not the change's
            // doing. What a change may not do is break it once more, for the
            // part it touches or for a neighbour it leaves stranded.
            let before = violations(&plain(&|i| infos[i].switch));
            let after = violations(&plain(&changed));
            if after > before {
                return false;
            }
        }
    }
    true
}

/// How often a run of parts breaks the spreading rule: a first part
/// without chords, and each part without chords that has no neighbour with
/// them.
fn violations(plain: &[bool]) -> usize {
    let mut count = 0;
    for (k, &is_plain) in plain.iter().enumerate() {
        if !is_plain {
            continue;
        }
        let before = k > 0 && !plain[k - 1];
        let after = k + 1 < plain.len() && !plain[k + 1];
        if k == 0 || !(before || after) {
            count += 1;
        }
    }
    count
}

/// A chorus sung again with the same words as the chorus before it becomes
/// the one-line reminder `\insertchorus*` (its chords may differ: the
/// reminder prints none). So does a full `\insertchorus`. A chorus that comes
/// back with new words keeps them.
fn chorus_reminders(text: &str, scan: &Scan, infos: &[PartInfo]) -> Vec<Proposal> {
    let choruses: Vec<usize> = (0..infos.len())
        .filter(|&i| infos[i].part.kind == "chorus")
        .collect();
    if choruses.iter().any(|&i| infos[i].part.conditional)
        || scan.chorus_repeats.iter().any(|(_, c)| *c)
    {
        return Vec::new();
    }

    // Every place a chorus is sung, typed or reprinted, for preferring the
    // later ones: the chorus is read in full first.
    let mut sung: Vec<usize> = choruses.iter().map(|&i| infos[i].part.line).collect();
    sung.extend(
        scan.chorus_repeats
            .iter()
            .map(|(span, _)| text[..span.start].matches('\n').count() + 1),
    );
    sung.sort();
    let score_at = |line: usize| {
        let k = sung.iter().filter(|&&l| l < line).count();
        REMINDER - early(k, sung.len())
    };

    let mut proposals = Vec::new();
    for pair in choruses.windows(2) {
        let (latest, repeat) = (&infos[pair[0]], &infos[pair[1]]);
        // The copy dropped would no longer feed a later chorus that replays
        // its chords.
        let replayed_later = choruses
            .iter()
            .filter(|&&i| i > pair[1])
            .any(|&i| infos[i].chords.is_none());
        // Nor would it be what a full `\insertchorus` after it reprints:
        // that would go back to the chorus before, with that one's chords.
        let next_chorus = choruses
            .iter()
            .find(|&&i| i > pair[1])
            .map_or(text.len(), |&i| infos[i].part.span.start);
        let reprinted_later = scan.chorus_repeats.iter().any(|(span, _)| {
            span.start >= repeat.part.span.end
                && span.start < next_chorus
                && !text[span.clone()].starts_with("\\insertchorus*")
        });
        if latest.part.number != repeat.part.number
            || latest.words != repeat.words
            || latest.words.trim().is_empty()
            || replayed_later
            || reprinted_later
        {
            continue;
        }
        let star = match repeat.part.number {
            Some(n) => format!("\\insertchorus*[{n}]"),
            None => "\\insertchorus*".to_string(),
        };
        let shown_rows = match repeat.switch {
            Chords::Shown => repeat.chorded_rows,
            _ => 0,
        };
        proposals.push(Proposal {
            id: format!("insertchorus*@{}", repeat.part.line),
            kind: "insertchorus*",
            stage: Stage::AllPapers,
            line: repeat.part.line,
            description: format!(
                "the chorus has the words of line {}: shorten it to \\insertchorus*",
                latest.part.line
            ),
            gain_lines: (repeat.lyric_rows + shown_rows).saturating_sub(1),
            score: score_at(repeat.part.line),
            edits: vec![Edit {
                span: repeat.part.span.clone(),
                replacement: star,
            }],
            effect: None,
        });
    }

    // A full reprint becomes the reminder of what it reprints.
    let latest_rows = |at: usize| {
        choruses
            .iter()
            .rev()
            .map(|&i| &infos[i])
            .find(|info| info.part.span.end <= at)
            .map(|info| info.lyric_rows + info.chorded_rows)
    };
    for (span, _) in &scan.chorus_repeats {
        let written = &text[span.clone()];
        if written.starts_with("\\insertchorus*") {
            continue;
        }
        let Some(rows) = latest_rows(span.start) else {
            continue;
        };
        let line = text[..span.start].matches('\n').count() + 1;
        proposals.push(Proposal {
            id: format!("insertchorus*@{line}"),
            kind: "insertchorus*",
            stage: Stage::AllPapers,
            line,
            description: "the chorus is reprinted in full: shorten it to \\insertchorus*"
                .to_string(),
            gain_lines: rows.saturating_sub(1),
            score: score_at(line),
            edits: vec![Edit {
                span: span.start..span.start + "\\insertchorus".len(),
                replacement: "\\insertchorus*".to_string(),
            }],
            effect: None,
        });
    }
    proposals
}

/// The last resort on small paper: the whole song a type size smaller
/// (`\smalleronsmallpage`, 8 pt for 9; anything smaller is too small to
/// play from). It goes in front of the first part, where the songbook's
/// songs have it.
fn smaller_type(text: &str, scan: &Scan, infos: &[PartInfo]) -> Vec<Proposal> {
    let already = text
        .lines()
        .map(code_of)
        .any(|code| code.contains("\\smalleronsmallpage"));
    let Some(first) = scan.parts.first() else {
        return Vec::new();
    };
    if already {
        return Vec::new();
    }
    let line_start = text[..first.span.start].rfind('\n').map_or(0, |at| at + 1);
    let rows: usize = infos
        .iter()
        .map(|i| {
            i.lyric_rows
                + if i.switch == Chords::Shown {
                    i.chorded_rows
                } else {
                    0
                }
        })
        .sum();
    vec![Proposal {
        id: format!("smalleronsmallpage@{}", first.line),
        kind: "smalleronsmallpage",
        stage: Stage::SmallPaper,
        line: first.line,
        description: "set the song a size smaller on small paper (8 pt): \\smalleronsmallpage"
            .to_string(),
        // 8 pt for 9 is about one line in nine. It also narrows every line,
        // so it can mend a line too wide on a song too short to save one.
        gain_lines: (rows / 9).max(1),
        score: SMALLER_TYPE,
        edits: vec![Edit {
            span: line_start..line_start,
            replacement: "\\smalleronsmallpage\n\n".to_string(),
        }],
        effect: None,
    }]
}

fn part_name(kind: &str) -> &str {
    match kind.trim_end_matches('*') {
        "chorus" => "chorus",
        "prechorus" => "pre-chorus",
        "bridge" => "bridge",
        "intro" => "intro",
        "outro" => "outro",
        _ => "verse",
    }
}

/// Applies the chosen proposals together. They must not overlap, and
/// together they must still keep chords spread out.
pub fn apply(text: &str, chosen: &[&Proposal]) -> Result<String, String> {
    let mut edits: Vec<&Edit> = chosen.iter().flat_map(|p| &p.edits).collect();
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    for pair in edits.windows(2) {
        if pair[1].span.start < pair[0].span.end
            || (pair[0].span == pair[1].span && pair[0].span.is_empty())
        {
            return Err(
                "the chosen proposals change the same text; apply one, then ask again".into(),
            );
        }
    }
    let effects: Vec<(usize, Chords)> = chosen.iter().filter_map(|p| p.effect).collect();
    if effects.len() > 1 {
        let scan = scan(text);
        let infos: Vec<PartInfo> = scan.parts.iter().map(|p| info(text, p)).collect();
        if !spread_out(&infos, &effects) {
            return Err(
                "together, the chosen proposals would leave parts with no chorded part next to them; apply one, then ask again"
                    .into(),
            );
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for edit in edits {
        out.push_str(&text[at..edit.span.start]);
        out.push_str(&edit.replacement);
        at = edit.span.end;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(body: &str) -> String {
        format!("\\beginsong{{T}}[\n    lang={{czech}},\n]\n\n{body}\\endsong\n")
    }

    fn part(head: &str, lines: &[&str]) -> String {
        let kind = head.trim_start_matches("\\begin");
        let kind = kind
            .split(['[', '\\'])
            .next()
            .unwrap()
            .trim_end_matches('*');
        let body: String = lines.iter().map(|l| format!("    {l}\n")).collect();
        format!("{head}\n{body}\\end{kind}\n\n")
    }

    const V: [&str; 2] = ["\\[C]Jedna \\[G]dvě", "\\[Am]tři \\[F]čtyři"];
    const W: [&str; 2] = ["\\[C]Pět \\[G]šest", "\\[Am]sedm \\[F]osm"];
    const R: [&str; 2] = ["\\[F]Refrén \\[G]zní", "\\[C]a zní"];

    fn ids(text: &str) -> Vec<String> {
        proposals(text).into_iter().map(|p| p.id).collect()
    }

    fn applied(text: &str, id: &str) -> String {
        let all = proposals(text);
        let chosen = all.iter().find(|p| p.id == id).expect(id);
        apply(text, &[chosen]).unwrap()
    }

    #[test]
    fn a_verse_repeating_the_chords_of_the_first_can_hide_them() {
        let text = song(&[part("\\beginverse", &V), part("\\beginverse", &W)].concat());
        let ids = ids(&text);
        assert_eq!(ids[0], "chordsoff@10");
        assert!(ids.contains(&"chordsoffonsmallpage@10".to_string()));
        let out = applied(&text, "chordsoff@10");
        assert!(
            out.contains("\\beginverse\\chordsoff\n    \\[C]Pět"),
            "{out}"
        );
        // Only those bytes changed.
        assert_eq!(out.replacen("\\chordsoff", "", 1), text);
    }

    #[test]
    fn the_first_verse_and_other_progressions_keep_their_chords() {
        let other = ["\\[D]Jiné \\[E]akordy", "\\[A]tady"];
        let text = song(&[part("\\beginverse", &V), part("\\beginverse", &other)].concat());
        assert!(ids(&text).iter().all(|id| !id.starts_with("chordsoff")));
    }

    #[test]
    fn chorded_verses_stay_spread_out() {
        // Four verses on one progression: the 2nd may go plain, and then
        // the 3rd, but not the 4th next to a plain 3rd.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse\\chordsoff", &W),
                part("\\beginverse\\chordsoff", &V),
                part("\\beginverse", &W),
            ]
            .concat(),
        );
        let ids = ids(&text);
        assert!(!ids.iter().any(|id| id == "chordsoff@20"), "{ids:?}");
        // Not on small paper either: the 3rd is plain there too.
        assert!(
            !ids.iter().any(|id| id == "chordsoffonsmallpage@20"),
            "{ids:?}"
        );

        // Two at once may not leave one stranded either.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginverse", &V),
            ]
            .concat(),
        );
        let all = proposals(&text);
        let two: Vec<&Proposal> = all.iter().filter(|p| p.kind == "chordsoff").collect();
        assert_eq!(two.len(), 2);
        assert!(apply(&text, &two).is_err());
    }

    #[test]
    fn a_change_may_not_strand_a_neighbour_already_plain() {
        // Verse 3 is plain and leans on verse 2; hiding verse 2's chords
        // would leave verse 3 without a chorded neighbour.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginverse\\chordsoff", &V),
            ]
            .concat(),
        );
        let ids = ids(&text);
        assert!(!ids.iter().any(|id| id.ends_with("@10")), "{ids:?}");
    }

    #[test]
    fn a_song_already_breaking_the_rule_can_still_take_a_harmless_change() {
        // Verses 2 and 3 are plain side by side, with verse 1 next to 2 only:
        // verse 3 already breaks the rule. Hiding verse 5's chords, with
        // verse 4 chorded beside it, adds no break.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse\\chordsoff", &W),
                part("\\beginverse\\chordsoff", &V),
                part("\\beginverse\\chordsoff", &W),
                part("\\beginverse", &V),
                part("\\beginverse", &W),
            ]
            .concat(),
        );
        let ids = ids(&text);
        assert!(ids.contains(&"chordsoff@30".to_string()), "{ids:?}");
        assert!(!ids.contains(&"chordsoff@25".to_string()), "{ids:?}");
    }

    #[test]
    fn a_part_off_on_small_paper_goes_off_everywhere_by_trading_the_word() {
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse\\chordsoffonsmallpage", &W),
            ]
            .concat(),
        );
        let out = applied(&text, "chordsoff@10");
        assert!(out.contains("\\beginverse\\chordsoff\n"), "{out}");
        assert!(!out.contains("onsmallpage"));
    }

    #[test]
    fn a_chorus_with_the_same_words_becomes_its_reminder() {
        let other_chords = ["\\[D]Refrén \\[E]zní", "\\[A]a zní"];
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginchorus", &other_chords),
            ]
            .concat(),
        );
        let out = applied(&text, "insertchorus*@15");
        assert!(
            out.ends_with("\\endverse\n\n\\insertchorus*\n\n\\endsong\n"),
            "{out}"
        );
    }

    #[test]
    fn apostrophes_and_punctuation_do_not_make_new_words() {
        let typed = ["\\[F]Tomorrow I'll \\[G]still be crying,", "\\[C]a zní"];
        let retyped = [
            "\\[F]Tomorrow Ill \\[G]still be crying",
            "\\[C]a zní \\[Am]",
        ];
        let text = song(
            &[
                part("\\beginchorus", &typed),
                part("\\beginchorus", &retyped),
            ]
            .concat(),
        );
        assert!(
            ids(&text).contains(&"insertchorus*@10".to_string()),
            "{:?}",
            ids(&text)
        );
    }

    #[test]
    fn a_chorus_a_full_reprint_repeats_keeps_its_chords() {
        // The second chorus has other chords, and `\insertchorus` after it
        // reprints it; shortened, the reprint would bring back the first
        // chorus's chords instead.
        let other_chords = ["\\[D]Refrén \\[E]zní", "\\[A]a zní"];
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginchorus", &other_chords),
                "\\insertchorus\n\n".to_string(),
            ]
            .concat(),
        );
        assert!(
            !ids(&text).contains(&"insertchorus*@10".to_string()),
            "{:?}",
            ids(&text)
        );
        // The reprint itself may still become the reminder.
        assert!(
            ids(&text).contains(&"insertchorus*@15".to_string()),
            "{:?}",
            ids(&text)
        );
    }

    #[test]
    fn a_chorus_with_new_words_keeps_them() {
        let new_words = ["\\[F]Jiný \\[G]refrén", "\\[C]a zní"];
        let text = song(&[part("\\beginchorus", &R), part("\\beginchorus", &new_words)].concat());
        assert!(!ids(&text).iter().any(|id| id.starts_with("insertchorus")));
    }

    #[test]
    fn a_full_reprint_can_become_the_reminder_and_a_rep_stays() {
        let text = song(
            &[
                part("\\beginchorus", &R),
                "\\insertchorus \\rep{2}\n\n".to_string(),
            ]
            .concat(),
        );
        let out = applied(&text, "insertchorus*@10");
        assert!(out.contains("\\insertchorus* \\rep{2}\n"), "{out}");
    }

    #[test]
    fn numbered_choruses_need_the_same_number() {
        let text = song(&[part("\\beginchorus[1]", &R), part("\\beginchorus[2]", &R)].concat());
        assert!(!ids(&text).iter().any(|id| id.starts_with("insertchorus")));
        let text = song(&[part("\\beginchorus[1]", &R), part("\\beginchorus[1]", &R)].concat());
        let out = applied(&text, "insertchorus*@10");
        assert!(out.contains("\\insertchorus*[1]\n"), "{out}");
    }

    #[test]
    fn smaller_type_goes_before_the_first_part_and_comes_last() {
        let text = song(
            &[
                "\\transpose{-2}\n\n".to_string(),
                part("\\beginverse", &V),
                part("\\beginverse", &W),
            ]
            .concat(),
        );
        let all = proposals(&text);
        let last = all.last().unwrap();
        assert_eq!(last.kind, "smalleronsmallpage");
        let out = apply(&text, &[last]).unwrap();
        assert!(
            out.contains("\\transpose{-2}\n\n\\smalleronsmallpage\n\n\\beginverse\n"),
            "{out}"
        );
        assert!(!ids(&out).iter().any(|id| id.starts_with("smaller")));
    }

    #[test]
    fn all_papers_come_before_small_paper() {
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginchorus", &R),
            ]
            .concat(),
        );
        let stages: Vec<Stage> = proposals(&text).iter().map(|p| p.stage).collect();
        let mut sorted = stages.clone();
        sorted.sort();
        assert_eq!(stages, sorted);
        // A glance back to the chorus reads better than chords looked up
        // elsewhere, line after line.
        let kinds: Vec<&str> = proposals(&text).iter().map(|p| p.kind).collect();
        assert_eq!(kinds[..2], ["insertchorus*", "chordsoff"]);
    }

    /// The first proposal, applied, again and again: what a person taking
    /// the recommended step each time ends up with.
    fn plain_verses(mut text: String, steps: usize) -> Vec<usize> {
        for _ in 0..steps {
            let first = proposals(&text)
                .into_iter()
                .find(|p| p.kind == "chordsoff")
                .expect("a chordsoff proposal");
            text = apply(&text, &[&first]).unwrap();
        }
        let scan = scan(&text);
        (1..)
            .zip(&scan.parts)
            .filter(|(_, p)| has_word(&text[p.opening_rest.clone()], "chordsoff"))
            .map(|(n, _)| n)
            .collect()
    }

    #[test]
    fn chords_go_off_alternating_and_late_first() {
        let verses = |n: usize| -> String {
            (0..n)
                .map(|k| part("\\beginverse", if k % 2 == 0 { &V } else { &W }))
                .collect()
        };
        // Four verses: the last first, then the second, never the third.
        assert_eq!(plain_verses(song(&verses(4)), 1), [4]);
        assert_eq!(plain_verses(song(&verses(4)), 2), [2, 4]);
        // Five: the last, then the third.
        assert_eq!(plain_verses(song(&verses(5)), 2), [3, 5]);

        // Then the second can only go next to the plain third: offered,
        // but rated lower and saying why.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginverse\\chordsoff", &V),
                part("\\beginverse", &W),
                part("\\beginverse\\chordsoff", &V),
            ]
            .concat(),
        );
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.id == "chordsoff@10")
            .unwrap();
        assert_eq!(p.grade(), "poor");
        assert!(
            p.description.contains("next to a verse already"),
            "{}",
            p.description
        );
    }

    #[test]
    fn a_neighbour_plain_on_small_paper_counts_for_every_paper() {
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse\\chordsoffonsmallpage", &W),
                part("\\beginverse", &V),
                part("\\beginverse", &W),
            ]
            .concat(),
        );
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.id == "chordsoff@15")
            .unwrap();
        assert_eq!(p.grade(), "poor");
    }

    #[test]
    fn being_early_never_changes_the_grade_by_itself() {
        let verses: String = (0..8)
            .map(|k| part("\\beginverse", if k % 2 == 0 { &V } else { &W }))
            .collect();
        let p = proposals(&song(&verses))
            .into_iter()
            .find(|p| p.id == "chordsoff@10")
            .unwrap();
        assert_eq!(p.grade(), "fair");

        let choruses: String = (0..8).map(|_| part("\\beginchorus", &R)).collect();
        let p = proposals(&song(&choruses))
            .into_iter()
            .find(|p| p.id == "insertchorus*@10")
            .unwrap();
        assert_eq!(p.grade(), "good");
    }

    #[test]
    fn every_proposal_has_its_rating() {
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginchorus", &R),
            ]
            .concat(),
        );
        // The second chorus can lose its chords as well as be shortened.
        let mut grades: Vec<(&str, &str)> = proposals(&text)
            .iter()
            .map(|p| (p.kind, p.grade()))
            .collect();
        grades.dedup();
        assert_eq!(
            grades,
            [
                ("insertchorus*", "good"),
                ("chordsoff", "fair"),
                ("chordsoffonsmallpage", "fair"),
                ("smalleronsmallpage", "last resort"),
            ]
        );
    }

    #[test]
    fn overlapping_proposals_are_refused_together() {
        let text = song(&[part("\\beginchorus", &R), part("\\beginchorus", &R)].concat());
        let all = proposals(&text);
        let both: Vec<&Proposal> = all
            .iter()
            .filter(|p| p.kind == "chordsoff" || p.kind == "insertchorus*")
            .collect();
        assert_eq!(both.len(), 2);
        assert!(apply(&text, &both).is_err());
    }
}
