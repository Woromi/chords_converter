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
//! another, line after line. Where chords go off matters too: the nearer a
//! part without chords is to the part before it whose chords it repeats,
//! the better (the reader looks back that far), and of two as near, the
//! chords stay towards the beginning. A smaller type costs the whole song,
//! so it comes last.

use super::{PartSpan, Scan, code_of, scan};
use std::ops::Range;

mod chordlines;
mod typos;

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

/// A part of the song, by its first line and its bytes from `\begin…` to
/// the end of `\end…`.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub line: usize,
    pub span: Range<usize>,
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
    /// The parts it relies on: where the chords it hides are still shown,
    /// or the chorus a reminder stands for.
    pub sources: Vec<Source>,
    /// The parts whose chord switch it changes, and to what, for checking
    /// that the proposals chosen together still keep chords spread out.
    effects: Vec<(usize, Chords)>,
    /// For joined lines, what it joins. Joining is offered only once the
    /// joined song is typeset and no line came out too wide, which needs
    /// these to tell a line too wide already.
    joins: Option<Joins>,
}

/// What a proposal joining lines joins: the lines (1-based, inclusive) of
/// its parts, whether they are choruses, and the kind in words.
#[derive(Debug, Clone, PartialEq)]
struct Joins {
    parts: Vec<(usize, usize)>,
    choruses: bool,
    name: String,
}

/// The readability a change leaves, before its place is taken into account.
/// Lines joined in pairs keep every word and every chord in its place:
/// nothing is looked up elsewhere.
const JOINED: u8 = 90;
const REMINDER: u8 = 80;
/// Rows played again printed once with `(×n)`: nothing is looked up
/// elsewhere, only counted.
const REPEATED_ROWS: u8 = 85;
const CHORDS_OFF: u8 = 70;
const SMALLER_TYPE: u8 = 20;
/// Taken off for each line between a part without chords and the nearest
/// part showing the same ones (decided with the user: what matters is how
/// far the reader looks for the chords). A verse right after the one whose
/// chords it repeats loses nothing; one after a four-line chorus, 8.
const PER_LINE_AWAY: u8 = 2;
/// A part's chords that cannot be found at all: worse than any distance.
const FAR: usize = 1000;
/// A part without chords and no neighbour of its kind with them: the last
/// resort, down to chords on the first of a kind alone (the user), above a
/// smaller type only.
const UNSPREAD: u8 = 30;
/// The lowest a change that keeps chords spread out goes: still "poor",
/// never "last resort".
const POOR: u8 = 35;
/// Taken off, at most, for a change on the first part that can take it,
/// less further on, so the words and chords stay towards the beginning.
/// Small enough that the place never changes a grade on its own.
const EARLY: u8 = 5;

impl Proposal {
    /// Whether it corrects the song rather than fitting a page: what
    /// `check` lists, and `shrink` leaves out.
    pub fn is_correction(&self) -> bool {
        matches!(
            self.kind,
            "restructure" | "typo" | "apostrophes" | "tie" | "comma" | "chordlines"
        )
    }

    /// What a program groups it under: `fit`, `restructure` or `typo`.
    pub fn category(&self) -> &'static str {
        match self.kind {
            "restructure" => "restructure",
            "typo" | "apostrophes" | "tie" | "comma" | "chordlines" => "typo",
            _ => "fit",
        }
    }

    /// Whether it joins lines, and so may be offered only once the joined
    /// song was typeset with no line too wide.
    pub fn needs_width_check(&self) -> bool {
        self.joins.is_some()
    }

    /// For joined lines, the kind of part they are joined in, in words.
    pub fn joined_kind(&self) -> Option<&str> {
        self.joins.as_ref().map(|j| j.name.as_str())
    }

    /// Whether a line the typesetter found too wide is one this proposal
    /// would make longer still: a line of a part it joins (a chorus's
    /// line is not known, only that it is in a chorus).
    pub fn lengthens(&self, line: Option<usize>, in_chorus: bool) -> bool {
        let Some(Joins {
            parts, choruses, ..
        }) = &self.joins
        else {
            return false;
        };
        match line {
            Some(line) => parts.iter().any(|&(a, b)| (a..=b).contains(&line)),
            None => in_chorus && *choruses,
        }
    }

    /// The rating in words: what a person reads next to the score.
    pub fn grade(&self) -> &'static str {
        if self.is_correction() {
            return "correction";
        }
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
    let names = Names::of(text, &scan);
    proposals.extend(chords_off(&names, &infos, Stage::AllPapers));
    proposals.extend(chorus_reminders(text, &scan, &infos, &names));
    proposals.extend(choruses_in_disguise(&infos, &names));
    proposals.extend(repeated_rows(text, &infos, &names));
    proposals.extend(typos::typos(&infos, &names, scan.lang.as_deref()));
    proposals.extend(chordlines::chords_across_lines(&infos, &names));
    proposals.extend(chords_off(&names, &infos, Stage::SmallPaper));
    proposals.extend(smaller_type(text, &scan, &infos));
    proposals.extend(joined_lines(&names, &infos, scan.lang.as_deref()).0);

    // Restructuring is worth it even when it saves nothing; a correction a
    // person marked right with `% nocheck` is not one.
    proposals.retain(|p| p.gain_lines > 0 || p.is_correction());
    proposals.retain(|p| !(p.is_correction() && marked_right(text, p)));
    let rank = |p: &Proposal| {
        (
            p.stage,
            std::cmp::Reverse(p.score),
            // The later of two as readable: chords and words stay towards
            // the beginning.
            std::cmp::Reverse(p.line),
            std::cmp::Reverse(p.gain_lines),
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
/// (`\chordsoffonsmallpage`). The earlier part has to show its chords, and
/// the parts of the kind have to stay spread out: the first shows its
/// chords, and every one without has a neighbour with.
///
/// A plain part reads its chords from the nearest part before it that
/// shows the same ones, so a layout costs the lines between, summed over
/// its plain parts (`layout_cost`, decided with the user). Choosing one
/// change at a time can lead into a corner: in 1 2 R 3 4 R 5 with verse 3
/// plain, hiding 2 sends verse 3 back past the chorus and verse 2. So every
/// layout with one more plain part is weighed as a whole, and when the best
/// one needs chords shown again somewhere, it is offered as one proposal
/// that hides some and shows others (hide 2 and 4, show 3).
fn chords_off(names: &Names, infos: &[PartInfo], stage: Stage) -> Vec<Proposal> {
    let mut kinds: Vec<&str> = infos
        .iter()
        .filter(|i| switchable(i))
        .map(|i| i.part.kind.as_str())
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
        .into_iter()
        .flat_map(|kind| chords_off_of_kind(names, infos, stage, kind))
        .collect()
}

/// More parts of a kind than this are only offered one change at a time.
const MAX_LAYOUT_PARTS: usize = 16;

fn chords_off_of_kind(
    names: &Names,
    infos: &[PartInfo],
    stage: Stage,
    kind: &str,
) -> Vec<Proposal> {
    let text = names.text;
    let (plain_state, word, id_kind) = match stage {
        Stage::AllPapers => (Chords::Off, "chordsoff", "chordsoff"),
        Stage::SmallPaper => (
            Chords::OffOnSmallPage,
            "chordsoffonsmallpage",
            "chordsoffonsmallpage",
        ),
    };
    let kin: Vec<usize> = (0..infos.len())
        .filter(|&j| infos[j].part.kind == kind && switchable(&infos[j]))
        .collect();
    // The parts whose switch may change: chords to show or hide, and on
    // small paper, not those hidden everywhere already.
    let variable: Vec<usize> = kin
        .iter()
        .copied()
        .filter(|&j| {
            infos[j].chords.as_ref().is_some_and(|c| !c.is_empty())
                && (stage == Stage::AllPapers || infos[j].switch != Chords::Off)
        })
        .collect();
    // What a variable part shows when not plain: a part off on small paper
    // only stays so on every paper; one off everywhere gets its chords back.
    let unplain = |j: usize| match (stage, infos[j].switch) {
        (Stage::AllPapers, Chords::OffOnSmallPage) => Chords::OffOnSmallPage,
        _ => Chords::Shown,
    };
    let current: u32 = variable
        .iter()
        .enumerate()
        .filter(|(_, j)| infos[**j].switch.plain_on(stage))
        .fold(0, |m, (b, _)| m | 1 << b);
    let size = current.count_ones() + 1;
    if size as usize > variable.len() {
        return Vec::new();
    }
    let masks: Vec<u32> = if variable.len() <= MAX_LAYOUT_PARTS {
        (0..1u32 << variable.len())
            .filter(|m| m.count_ones() == size)
            .collect()
    } else {
        (0..variable.len())
            .map(|b| current | 1 << b)
            .filter(|m| m.count_ones() == size)
            .collect()
    };

    let state_in = |mask: u32, j: usize| match variable.iter().position(|&v| v == j) {
        Some(b) if mask & 1 << b != 0 => plain_state,
        Some(_) => unplain(j),
        None => infos[j].switch,
    };
    let has_chords = |j: usize| infos[j].chords.as_ref().is_some_and(|c| !c.is_empty());
    // The chord rows small paper hides: a layout may not hide fewer, or
    // fitting the page would make small paper longer.
    let hidden_small = |mask: u32| -> usize {
        kin.iter()
            .filter(|&&j| state_in(mask, j).plain_on(Stage::SmallPaper))
            .map(|&j| infos[j].chorded_rows)
            .sum()
    };

    // How far apart two parts are: the lines printed between them.
    let lines_before = &names.rows_before;
    let end_line = |j: usize| text[..infos[j].part.span.end].matches('\n').count() + 1;
    let gap = |first: usize, second: usize| {
        let from = end_line(first).min(lines_before.len() - 1);
        let to = (infos[second].part.line - 1).min(lines_before.len() - 1);
        lines_before[to].saturating_sub(lines_before[from])
    };
    // Where a plain part's chords are read on a paper: the nearest part of
    // the kind before it with the same chords that shows them there. The
    // reader looks back to chords already played, not ahead.
    let nearest = |mask: u32, j: usize, paper: Stage| {
        kin.iter()
            .copied()
            .filter(|&q| {
                q < j && plays_chords_of(&infos[q], &infos[j]) && !state_in(mask, q).plain_on(paper)
            })
            .min_by_key(|&q| gap(q, j))
    };
    // A layout's cost on each paper: for each part plain there, how far its
    // chords are (decided with the user: the reader looks them up, so the
    // nearer the better).
    let layout_cost = |mask: u32| -> [usize; 2] {
        let mut cost = [0; 2];
        for (p, paper) in [Stage::AllPapers, Stage::SmallPaper]
            .into_iter()
            .enumerate()
        {
            for &j in &kin {
                if has_chords(j) && state_in(mask, j).plain_on(paper) {
                    cost[p] += nearest(mask, j, paper).map_or(FAR, |q| gap(q, j));
                }
            }
        }
        cost
    };
    // A newly plain part repeats chords shown before it.
    let source_before = |mask: u32, j: usize| {
        (0..j).any(|q| {
            infos[q].part.kind == kind
                && plays_chords_of(&infos[q], &infos[j])
                && state_in(mask, q) == Chords::Shown
        })
    };

    struct Layout {
        mask: u32,
        /// On A4 and on A5.
        cost: [usize; 2],
        /// Where the plain parts are, summed: of two layouts as good, the
        /// one with its chords nearer the beginning.
        later: usize,
        /// Per part it changes: the part and its new switch.
        changes: Vec<(usize, Chords)>,
        /// Whether it leaves a part without chords and no neighbour of its
        /// kind with them: allowed, but only as the last resort.
        unspread: bool,
    }
    let small_before = hidden_small(current);
    let mut layouts = Vec::new();
    'mask: for mask in masks {
        let mut changes = Vec::new();
        for &j in &variable {
            let state = state_in(mask, j);
            if state == infos[j].switch {
                continue;
            }
            if state.plain_on(stage) && !source_before(mask, j) {
                continue 'mask;
            }
            changes.push((j, state));
        }
        if hidden_small(mask) < small_before {
            continue;
        }
        let unspread = !spread_out(infos, &changes);
        let later = kin
            .iter()
            .enumerate()
            .filter(|(_, j)| state_in(mask, **j).plain_on(stage))
            .map(|(k, _)| k)
            .sum();
        layouts.push(Layout {
            mask,
            cost: layout_cost(mask),
            later,
            changes,
            unspread,
        });
    }
    let current_cost = layout_cost(current);

    let proposal = |layout: &Layout| -> Proposal {
        let hidden: Vec<usize> = layout
            .changes
            .iter()
            .filter(|c| c.1.plain_on(stage))
            .map(|c| c.0)
            .collect();
        let shown: Vec<(usize, Chords)> = layout
            .changes
            .iter()
            .copied()
            .filter(|c| !c.1.plain_on(stage))
            .collect();
        let line = |j: usize| infos[j].part.line;
        let mut id = format!(
            "{id_kind}@{}",
            hidden
                .iter()
                .map(|&j| line(j).to_string())
                .collect::<Vec<_>>()
                .join("+")
        );
        for (j, _) in &shown {
            id.push_str(&format!("-{}", line(*j)));
        }
        // How much further the reader looks for chords, on the paper where
        // it is further; and of two as near, the later is better.
        let further = (0..2)
            .map(|p| layout.cost[p].saturating_sub(current_cost[p]))
            .max()
            .unwrap();
        let score = if layout.unspread {
            UNSPREAD
        } else {
            (CHORDS_OFF as usize)
                .saturating_sub(PER_LINE_AWAY as usize * further)
                .max(POOR as usize) as u8
        };
        let gain = hidden
            .iter()
            .map(|&j| infos[j].chorded_rows)
            .sum::<usize>()
            .saturating_sub(
                shown
                    .iter()
                    .map(|(j, _)| infos[*j].chorded_rows)
                    .sum::<usize>(),
            );
        let edits = layout
            .changes
            .iter()
            .map(|&(j, to)| switch_edit(text, infos[j].part, infos[j].switch, to))
            .collect();
        let mut sources: Vec<usize> = hidden
            .iter()
            .filter_map(|&j| nearest(layout.mask, j, stage))
            .collect();
        sources.sort();
        sources.dedup();
        let description = if shown.is_empty() {
            let j = hidden[0];
            let q = nearest(layout.mask, j, stage).unwrap();
            let away = match gap(q, j) {
                0 => String::new(),
                1 => " (1 line away)".to_string(),
                n => format!(" ({n} lines away)"),
            };
            let last_resort = if layout.unspread {
                format!(
                    "; it leaves a {} without chords and none beside it with them: the last resort",
                    part_name(kind)
                )
            } else {
                String::new()
            };
            format!(
                "{} repeats the chords of {}{away}: \\{word}{last_resort}",
                names.parts[j], names.parts[q],
            )
        } else {
            let list = |js: &[usize]| {
                let names: Vec<&str> = js.iter().map(|&j| names.parts[j].as_str()).collect();
                match names.split_last() {
                    Some((last, [])) => last.to_string(),
                    Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                    None => String::new(),
                }
            };
            let paper = if stage == Stage::AllPapers
                && shown.iter().all(|c| c.1 == Chords::OffOnSmallPage)
            {
                " on large paper"
            } else {
                ""
            };
            let shown: Vec<usize> = shown.iter().map(|c| c.0).collect();
            format!(
                "hide the chords of {} and show them again in {}{paper}, so the {}s without chords are nearer to theirs: \\{word}",
                list(&hidden),
                list(&shown),
                part_name(kind)
            )
        };
        Proposal {
            id,
            kind: id_kind,
            stage,
            line: line(hidden[0]),
            description,
            gain_lines: gain,
            score,
            edits,
            effects: layout.changes.clone(),
            sources: sources.iter().map(|&q| source_of(infos[q].part)).collect(),
            joins: None,
        }
    };

    // Every single change, and the best layout overall when it needs chords
    // shown again and reads better than any single change.
    let singles: Vec<&Layout> = layouts
        .iter()
        .filter(|l| l.mask & current == current)
        .collect();
    let mut proposals: Vec<Proposal> = singles.iter().map(|l| proposal(l)).collect();
    let total = |l: &Layout| l.cost[0] + l.cost[1];
    // Spread out first, then by distance.
    let rank = |l: &Layout| (l.unspread, total(l));
    let best_single = singles.iter().map(|l| rank(l)).min();
    let best = layouts
        .iter()
        .filter(|l| l.mask & current != current)
        .min_by_key(|l| (rank(l), std::cmp::Reverse(l.later)));
    if let Some(best) = best
        && best_single.is_none_or(|c| rank(best) < c)
    {
        proposals.push(proposal(best));
    }
    proposals
}

/// Whether a part plays chords an earlier part shows, so they can be read
/// there: the same chords, "no chord" marks aside (they name nothing to
/// play), or the start of the earlier part's, for a part cut short (a last
/// chorus of three lines). A chord played otherwise is not the same.
fn plays_chords_of(earlier: &PartInfo, part: &PartInfo) -> bool {
    let played = |info: &PartInfo| -> Option<Vec<String>> {
        info.chords.as_ref().map(|chords| {
            chords
                .iter()
                .filter(|c| c.as_str() != "\\nochord")
                .cloned()
                .collect()
        })
    };
    let (Some(earlier), Some(part)) = (played(earlier), played(part)) else {
        return false;
    };
    !part.is_empty() && (part == earlier || (part.len() >= 2 && earlier.starts_with(&part)))
}

/// The edit that turns a part's chord switch from one state to another.
fn switch_edit(text: &str, part: &PartSpan, from: Chords, to: Chords) -> Edit {
    let opening = &part.opening_rest;
    let word = |c: Chords| match c {
        Chords::Off => "\\chordsoff",
        Chords::OffOnSmallPage => "\\chordsoffonsmallpage",
        Chords::Shown => "",
    };
    let span = match from {
        Chords::Shown => opening.start..opening.start,
        _ => {
            let w = word(from);
            let at = text[opening.clone()]
                .match_indices(w)
                .map(|(at, _)| opening.start + at)
                .find(|&at| {
                    !text[at + w.len()..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic())
                })
                .expect("the switch the part was read with");
            at..at + w.len()
        }
    };
    Edit {
        span,
        replacement: word(to).to_string(),
    }
}

/// A part a proposal relies on, for a program to point at.
fn source_of(part: &PartSpan) -> Source {
    Source {
        line: part.line,
        span: part.span.clone(),
    }
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
/// the one-line reminder `\insertchorus*`, when its chords are the same give
/// or take one (`chords_close`): the reminder prints none, so the player
/// plays the chorus before. So does a full `\insertchorus`. A chorus that
/// comes back with new words or other chords keeps them.
fn chorus_reminders(text: &str, scan: &Scan, infos: &[PartInfo], names: &Names) -> Vec<Proposal> {
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
            || !chords_close(latest, repeat)
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
                "{} has the words of {}: shorten it to \\insertchorus*",
                names.parts[pair[1]], names.parts[pair[0]]
            ),
            gain_lines: (repeat.lyric_rows + shown_rows).saturating_sub(1),
            score: score_at(repeat.part.line),
            edits: vec![Edit {
                span: repeat.part.span.clone(),
                replacement: star,
            }],
            effects: Vec::new(),
            sources: vec![source_of(latest.part)],
            joins: None,
        });
    }

    // A full reprint becomes the reminder of what it reprints.
    let latest_before = |at: usize| {
        choruses
            .iter()
            .rev()
            .map(|&i| &infos[i])
            .find(|info| info.part.span.end <= at)
    };
    for (r, (span, _)) in scan.chorus_repeats.iter().enumerate() {
        let written = &text[span.clone()];
        if written.starts_with("\\insertchorus*") {
            continue;
        }
        let Some(latest) = latest_before(span.start) else {
            continue;
        };
        let rows = latest.lyric_rows + latest.chorded_rows;
        let line = text[..span.start].matches('\n').count() + 1;
        proposals.push(Proposal {
            id: format!("insertchorus*@{line}"),
            kind: "insertchorus*",
            stage: Stage::AllPapers,
            line,
            description: format!(
                "{} is reprinted in full: shorten it to \\insertchorus*",
                names.repeats[r]
            ),
            gain_lines: rows.saturating_sub(1),
            score: score_at(line),
            edits: vec![Edit {
                span: span.start..span.start + "\\insertchorus".len(),
                replacement: "\\insertchorus*".to_string(),
            }],
            effects: Vec::new(),
            sources: vec![source_of(latest.part)],
            joins: None,
        });
    }
    proposals
}

/// Every two lines of every part of a kind joined into one (CLAUDE.md
/// "Fitting the page", lever 1): fewer, longer lines, all of the same
/// musical length again, and every word and chord kept. Only when it can be
/// done to every part of the kind alike and nothing between the lines would
/// change its meaning joined. A part of an odd number of lines keeps its
/// last line alone, which is how a verse of two half phrases and a whole one
/// comes out even ("I walk along the river / Down to the old mill / Waiting
/// for the morning train to come"), but only when every part of the kind has that
/// number, so they all come out alike. Whether the joined lines fit the
/// column is for the typesetter to say, so `join` offers it only after
/// measuring (`needs_width_check`).
///
/// Besides the proposals, it says for each kind it could not join why not,
/// so a person is not left to guess whether the lines were too wide.
fn joined_lines(
    names: &Names,
    infos: &[PartInfo],
    lang: Option<&str>,
) -> (Vec<Proposal>, Vec<String>) {
    let text = names.text;
    let mut kinds: Vec<&str> = infos.iter().map(|i| i.part.kind.as_str()).collect();
    kinds.sort();
    kinds.dedup();

    let mut proposals = Vec::new();
    let mut refused = Vec::new();
    let case = typos::capitals(text, infos);
    'kind: for kind in kinds {
        // An instrumental's rows join bar to bar, and a row with its own
        // `\rep` would come to repeat both.
        let bars = kind.starts_with("instrumental");
        let name = if kind == "verse*" {
            "unnumbered verse".to_string()
        } else {
            part_name(kind)
        };
        let mut refuse = |why: String| refused.push(format!("{name}: {why}"));
        let parts: Vec<(usize, &PartInfo)> = infos
            .iter()
            .enumerate()
            .filter(|(_, i)| i.part.kind == kind)
            .collect();

        // Each part's sung lines (byte range, without the newline) and its
        // `% nocheck` lines (whole, with the newline: they go along above
        // the joined line).
        let mut lines_of = Vec::new();
        for &(i, info) in &parts {
            let part = &names.parts[i];
            if info.part.conditional {
                refuse(format!("{part} is printed on one paper only (\\if…)"));
                continue 'kind;
            }
            let body = info.part.body.clone();
            let mut sung: Vec<Range<usize>> = Vec::new();
            let mut marks: Vec<Range<usize>> = Vec::new();
            let mut at = body.start;
            for line in text[body.clone()].split_inclusive('\n') {
                let range = at..at + line.trim_end_matches(['\n', '\r']).len();
                let whole = at..at + line.len();
                at += line.len();
                if is_nocheck_mark(&text[range.clone()]) {
                    marks.push(whole);
                    continue;
                }
                if text[range.clone()].trim().is_empty() {
                    // Before the first line and after the last, the
                    // layout's own; between lines, a gap that means one.
                    if sung.is_empty() {
                        continue;
                    }
                    if text[at..body.end].trim().is_empty() {
                        break;
                    }
                    refuse(format!(
                        "{part} has an empty line at line {}",
                        line_at(text, range.start)
                    ));
                    continue 'kind;
                }
                sung.push(range);
            }
            if sung.is_empty() {
                refuse(format!("{part} has no lines"));
                continue 'kind;
            }
            lines_of.push((part, sung, marks));
        }

        let mut counts: Vec<usize> = lines_of.iter().map(|(_, sung, _)| sung.len()).collect();
        counts.sort();
        counts.dedup();
        let odd = counts.iter().any(|n| n % 2 == 1);
        if odd && counts.len() > 1 {
            let counts: Vec<String> = counts.iter().map(|n| n.to_string()).collect();
            refuse(format!(
                "the parts have {} lines; an odd number pairs only when every part has the same",
                list_words(&counts)
            ));
            continue;
        }
        if counts == [1] {
            refuse(if parts.len() == 1 {
                "one line, nothing to pair".to_string()
            } else {
                "one line each, nothing to pair".to_string()
            });
            continue;
        }
        // The line left alone has to be about as long as a joined pair, or
        // the lines come out unequal after all (a chorus ending in a short
        // hook sung twice). Measured in letters sung, over all the parts.
        if odd && !bars {
            let letters = |r: &Range<usize>| lyric_words(code_of(&text[r.clone()])).chars().count();
            let (mut pairs, mut paired, mut alone) = (0, 0, 0);
            for (_, sung, _) in &lines_of {
                let (last, rest) = sung.split_last().expect("not empty");
                alone += letters(last);
                paired += rest.iter().map(letters).sum::<usize>();
                pairs += rest.len() / 2;
            }
            let parts = lines_of.len();
            // alone / parts < 0.6 * paired / pairs, in whole numbers.
            if 10 * alone * pairs < 6 * paired * parts {
                refuse(format!(
                    "the last of its {} lines is much shorter than two joined, so the lines would come out unequal",
                    counts[0]
                ));
                continue;
            }
        }

        let mut edits = Vec::new();
        let mut gain = 0;
        let mut pairs = 0;
        // Lines a comma went after, and lines that may want one.
        let mut commas: Vec<usize> = Vec::new();
        // Line starts made lowercase, and the words of those the song
        // nowhere else writes lowercase.
        let mut lowered = 0;
        let mut unsure_case: Vec<String> = Vec::new();
        let mut unsure: Vec<usize> = Vec::new();
        let mut spans = Vec::new();
        for (&(_, info), (part, sung, marks)) in parts.iter().zip(&lines_of) {
            for (k, line) in sung.iter().enumerate() {
                let raw = &text[line.clone()];
                let code = code_of(raw);
                let last = k + 1 == sung.len();
                // A line with a line joined after it: the last of an odd
                // number stays alone.
                let joined_after = k % 2 == 0 && !last;
                let number = line_at(text, line.start);
                let why = if code.trim().is_empty() {
                    Some(format!("line {number} of {part} is only a comment"))
                } else if joined_after && code.len() != raw.len() {
                    Some(format!(
                        "line {number} ends in a comment, which would swallow the line joined after it"
                    ))
                } else if let Some(word) =
                    ["textnote", "chordson", "chordsoff", "chordsoffonsmallpage"]
                        .iter()
                        .find(|w| has_word(code, w))
                        .map(|w| format!("\\{w}"))
                        .or_else(|| {
                            ["\\if", "\\else", "\\fi"]
                                .iter()
                                .find(|w| code.contains(**w))
                                .map(|w| format!("{w}…"))
                        })
                {
                    Some(format!(
                        "line {number} holds {word}, which needs a line of its own"
                    ))
                } else if (!last || bars) && has_word(code, "rep") {
                    Some(format!(
                        "line {number} is repeated (\\rep), and joined, its neighbour would be too"
                    ))
                } else {
                    None
                };
                if let Some(why) = why {
                    refuse(why);
                    continue 'kind;
                }
            }
            for pair in sung.chunks(2).filter(|pair| pair.len() == 2) {
                let first = text[pair[0].clone()].trim_end();
                let end = pair[0].start + first.len();
                let next = &text[pair[1].clone()];
                let mut start = pair[1].start + (next.len() - next.trim_start().len());
                // `| a | b |` and `| c | d |` meet at one bar line.
                if bars && first.ends_with('|') && text[start..].starts_with('|') {
                    start += 1;
                    start += text[start..].len() - text[start..].trim_start().len();
                }
                // A comma where the language's rule wants one; where none
                // tells, the place is named for a person to look at.
                if !bars
                    && let Some((edit, word, sure)) =
                        case.lowered(text, pair[0].clone(), pair[1].clone(), lang)
                {
                    edits.push(edit);
                    lowered += 1;
                    if !sure && !unsure_case.contains(&word) {
                        unsure_case.push(word);
                    }
                }
                if !bars && let Some(at) = bare_end(text, pair[0].clone()) {
                    let next_word = lyric_words(code_of(next));
                    let next_word = next_word.split_whitespace().next().unwrap_or("");
                    match typos::comma_before(lang, next_word) {
                        Some(true) => {
                            edits.push(Edit {
                                span: at..at,
                                replacement: ",".to_string(),
                            });
                            commas.push(line_at(text, pair[0].start));
                        }
                        Some(false) => {}
                        None => unsure.push(line_at(text, pair[0].start)),
                    }
                }
                edits.push(Edit {
                    span: end..start,
                    replacement: " ".to_string(),
                });
                // A mark between the two lines goes above them.
                let between: String = marks
                    .iter()
                    .filter(|m| m.start >= end && m.end <= start)
                    .map(|m| text[m.clone()].to_string())
                    .collect();
                if !between.is_empty() {
                    edits.push(Edit {
                        span: pair[0].start..pair[0].start,
                        replacement: between,
                    });
                }
                let chorded = |r: &Range<usize>| !chord_names(code_of(&text[r.clone()])).is_empty();
                gain += 1;
                // Two chord rows over two lyric rows become one of each; an
                // instrumental's row is its chords alone.
                if !bars && info.switch == Chords::Shown && chorded(&pair[0]) && chorded(&pair[1]) {
                    gain += 1;
                }
                pairs += 1;
            }
            let first = text[..info.part.span.start].matches('\n').count() + 1;
            let last = text[..info.part.span.end].matches('\n').count() + 1;
            spans.push((first, last));
        }
        let Some((_, first)) = parts.first() else {
            continue;
        };
        let alone = match counts.as_slice() {
            [n] if n % 2 == 1 => format!(", the last of its {n} lines staying alone"),
            _ => String::new(),
        };
        proposals.push(Proposal {
            id: format!("joinlines@{}", first.part.line),
            kind: "joinlines",
            stage: Stage::AllPapers,
            line: first.part.line,
            description: format!(
                "join every two lines of each {name} into one{alone} ({} lines into {pairs}){}{}{}",
                pairs * 2,
                match (lowered, unsure_case.as_slice()) {
                    (0, _) => String::new(),
                    (n, []) => format!("; {n} line start{} made lowercase", if n == 1 { "" } else { "s" }),
                    (n, words) => format!(
                        "; {n} line start{} made lowercase, {} though the song nowhere else writes {} so: check {} a name",
                        if n == 1 { "" } else { "s" },
                        list_words(words),
                        if words.len() == 1 { "it" } else { "them" },
                        if words.len() == 1 { "it is not" } else { "none is" },
                    ),
                },
                match commas.as_slice() {
                    [] => String::new(),
                    lines => format!("; a comma at the end of {}", list_lines(lines)),
                },
                match unsure.as_slice() {
                    [] => String::new(),
                    lines => format!(
                        "; {} may want a comma at {} end",
                        list_lines(lines),
                        if lines.len() == 1 { "its" } else { "their" }
                    ),
                }
            ),
            gain_lines: gain,
            score: JOINED,
            edits,
            effects: Vec::new(),
            sources: Vec::new(),
            joins: Some(Joins {
                parts: spans,
                choruses: kind == "chorus",
                name,
            }),
        });
    }
    (proposals, refused)
}

/// Why no kind of part had lines to join, kind by kind (`join`'s answer
/// when it offers nothing for a kind): the reasons found before typesetting.
pub fn unjoinable(text: &str) -> Vec<String> {
    let scan = scan(text);
    let infos: Vec<PartInfo> = scan.parts.iter().map(|p| info(text, p)).collect();
    let names = Names::of(text, &scan);
    joined_lines(&names, &infos, scan.lang.as_deref()).1
}

/// A part that may be the chorus sung again but typed as something else (a
/// site marked a modulated chorus "key change", and the converter made it
/// an unnumbered verse): the same words as the latest chorus. Only a person
/// can tell, since the same words need not be the same melody, so each such
/// part is its own proposal, and all it does is make the part a chorus,
/// with its own chords (decided with the user). The rest follows from the
/// other rules, one step at a time: `tidy` makes a chorus typed out again
/// `\insertchorus`, and `suggest` offers its reminder. It changes what the
/// parts are, not only how long the song is, so it is a kind of its own,
/// `restructure`, for a program to mark apart.
fn choruses_in_disguise(infos: &[PartInfo], names: &Names) -> Vec<Proposal> {
    let text = names.text;
    let mut proposals = Vec::new();
    let mut latest: Option<usize> = None;
    for (i, info) in infos.iter().enumerate() {
        if info.part.kind == "chorus" {
            latest = Some(i);
            continue;
        }
        let Some(chorus) = latest else { continue };
        if info.part.kind.starts_with("instrumental")
            || info.part.conditional
            || infos[chorus].part.conditional
            || infos[chorus].words.trim().is_empty()
            || info.words != infos[chorus].words
        {
            continue;
        }
        // The opening macro becomes the chorus's, its setup kept, and the
        // closing one with it.
        let bracket = infos[chorus]
            .part
            .number
            .map_or(String::new(), |n| format!("[{n}]"));
        let end = text[..info.part.span.end].rfind("\\end").unwrap();
        proposals.push(Proposal {
            id: format!("restructure@{}", info.part.line),
            kind: "restructure",
            stage: Stage::AllPapers,
            line: info.part.line,
            description: format!(
                "{} has the words of {}: if it is the chorus sung again, make it a chorus",
                names.parts[i], names.parts[chorus]
            ),
            gain_lines: 0,
            score: 0,
            edits: vec![
                Edit {
                    span: info.part.span.start..info.part.opening_rest.start,
                    replacement: format!("\\beginchorus{bracket}"),
                },
                Edit {
                    span: end..info.part.span.end,
                    replacement: "\\endchorus".to_string(),
                },
            ],
            effects: Vec::new(),
            sources: vec![source_of(infos[chorus].part)],
            joins: None,
        });
    }
    proposals
}

/// Whether a chorus sung again plays the chords of the one before, so its
/// reminder, which prints none, leaves nothing out: the same chords, give or
/// take one added, dropped or changed (a site's slip, as in Forever And One).
/// A chorus in another key, or played otherwise, keeps its own (the user).
/// A chorus that replays its chords with `^` plays the same ones.
fn chords_close(latest: &PartInfo, repeat: &PartInfo) -> bool {
    let (Some(a), Some(b)) = (&latest.chords, &repeat.chords) else {
        return true;
    };
    // Edit distance, one row at a time.
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut next = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            let cost = usize::from(x != y);
            next.push((row[j] + cost).min(row[j + 1] + 1).min(next[j] + 1));
        }
        row = next;
    }
    row[b.len()] <= 1
}

/// An instrumental's row played several times in a row is printed once,
/// with `\rep{n}` at its end (suggested by the user): `| \[Am]\[] | … |`
/// three times becomes `| \[Am]\[] | … | \rep{3}`. A row that already
/// ends with `\rep{k}` counts k times. Rows that replay chords with `^`
/// are left alone: each `^` plays the next chord remembered, so two rows
/// written alike play different chords.
fn repeated_rows(text: &str, infos: &[PartInfo], names: &Names) -> Vec<Proposal> {
    // A row's code without its own `\rep{k}`, and k.
    let counted = |code: &str| -> (String, usize) {
        let code = code.trim();
        if let Some(head) = code.strip_suffix('}')
            && let Some(at) = head.rfind("\\rep{")
            && let Ok(k) = head[at + 5..].trim().parse::<usize>()
        {
            return (head[..at].trim_end().to_string(), k);
        }
        (code.to_string(), 1)
    };
    let mut proposals = Vec::new();
    for (i, info) in infos.iter().enumerate() {
        if !info.part.kind.starts_with("instrumental") || info.part.conditional {
            continue;
        }
        // The rows: byte range of the line (no newline), its code, its count.
        let body = info.part.body.clone();
        let mut rows: Vec<(Range<usize>, String, usize, bool)> = Vec::new();
        let mut at = body.start;
        for raw in text[body.clone()].split_inclusive('\n') {
            let line = raw.trim_end_matches(['\n', '\r']);
            let code = code_of(line);
            let commented = code.len() != line.len();
            if !code.trim().is_empty() {
                let (base, k) = counted(code);
                rows.push((at..at + line.len(), base, k, commented));
            }
            at += raw.len();
        }
        let mut edits = Vec::new();
        let mut gain = 0;
        let mut first_line = None;
        let mut k = 0;
        while k < rows.len() {
            let (span, base, _, commented) = &rows[k];
            let mut end = k + 1;
            while end < rows.len() && rows[end].1 == *base && !rows[end].3 {
                end += 1;
            }
            if end - k > 1 && !commented && !base.contains('^') {
                let total: usize = rows[k..end].iter().map(|r| r.2).sum();
                let line = &text[span.clone()];
                let indent = &line[..line.len() - line.trim_start().len()];
                edits.push(Edit {
                    span: span.start..rows[end - 1].0.end,
                    replacement: format!("{indent}{base} \\rep{{{total}}}"),
                });
                gain += end - k - 1;
                first_line.get_or_insert(text[..span.start].matches('\n').count() + 1);
            }
            k = end;
        }
        let Some(line) = first_line else { continue };
        proposals.push(Proposal {
            id: format!("rep@{line}"),
            kind: "rep",
            stage: Stage::AllPapers,
            line,
            description: format!(
                "{} plays a row again right after itself: print it once with \\rep{{n}}",
                names.parts[i]
            ),
            gain_lines: gain,
            score: REPEATED_ROWS,
            edits,
            effects: Vec::new(),
            sources: Vec::new(),
            joins: None,
        });
    }
    proposals
}

/// Where a comma would go at the end of a line whose words end bare (no
/// punctuation): right after its last letter, before any chords after it.
fn bare_end(text: &str, line: Range<usize>) -> Option<usize> {
    let mut code = code_of(&text[line.clone()]).trim_end();
    // Chords after the last word, and the spaces between them.
    while code.ends_with(']')
        && let Some(open) = code.rfind("\\[")
    {
        code = code[..open].trim_end();
    }
    let last = code.chars().next_back()?;
    last.is_alphanumeric().then(|| line.start + code.len())
}

fn line_at(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// "line 12", "lines 12 and 30", "lines 12, 30 and 41".
fn list_lines(lines: &[usize]) -> String {
    let words: Vec<String> = lines.iter().map(usize::to_string).collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => format!("line {one}"),
        _ => format!("lines {}", list_words(&words)),
    }
}

/// "3", "3 and 5", "3, 4 and 5".
fn list_words(words: &[String]) -> String {
    match words.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Whether a person marked a finding right: `% nocheck` on the line above
/// the one it is about, or `% nocheck <kind>` anywhere in the song.
fn marked_right(text: &str, p: &Proposal) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let above = p.line.checked_sub(2).and_then(|i| lines.get(i));
    let mark = |line: &str| -> Option<String> {
        is_nocheck_mark(line).then(|| {
            line.trim()[1..].trim_start()["nocheck".len()..]
                .trim()
                .to_string()
        })
    };
    above
        .and_then(|l| mark(l))
        .is_some_and(|kind| kind.is_empty())
        || lines
            .iter()
            .any(|l| mark(l).is_some_and(|kind| kind == p.kind))
}

/// The edit that marks a finding right, so `check` leaves it out from then
/// on: `% nocheck` on a line of its own above the line it is about, or for
/// a finding about the whole song (apostrophes, ties) `% nocheck <kind>`
/// before its first part. `None` for a proposal that is no correction.
pub fn dismissal(text: &str, p: &Proposal) -> Option<Edit> {
    if !p.is_correction() {
        return None;
    }
    let line_start = |line: usize| -> usize {
        text.split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum()
    };
    if matches!(p.kind, "apostrophes" | "tie") {
        let first = scan(text).parts.first()?.line;
        return Some(Edit {
            span: line_start(first)..line_start(first),
            replacement: format!("% nocheck {}\n", p.kind),
        });
    }
    let at = line_start(p.line);
    let line = text[at..].lines().next().unwrap_or("");
    let indent = &line[..line.len() - line.trim_start().len()];
    Some(Edit {
        span: at..at,
        replacement: format!("{indent}% nocheck\n"),
    })
}

/// A line of its own that tells `check` a finding is right: `% nocheck`
/// for the line below it, `% nocheck <kind>` for the kind in the whole song.
pub(super) fn is_nocheck_mark(line: &str) -> bool {
    line.trim()
        .strip_prefix('%')
        .is_some_and(|rest| rest.trim_start().starts_with("nocheck"))
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
        effects: Vec::new(),
        sources: Vec::new(),
        joins: None,
    }]
}

/// What a person calls each part, for the descriptions: "verse 3", as the
/// page prints it, and "the 2nd chorus" for the parts printed without a
/// number. A chorus reprinted with `\insertchorus` counts as sung again.
struct Names<'a> {
    text: &'a str,
    /// One per part of the scan.
    parts: Vec<String>,
    /// One per `\insertchorus…` of the scan.
    repeats: Vec<String>,
    /// Per line of the song, the lines printed before it: a sung line is
    /// one, and a full `\insertchorus` the whole chorus it reprints.
    rows_before: Vec<usize>,
}

impl<'a> Names<'a> {
    fn of(text: &'a str, scan: &Scan) -> Self {
        // Every part and reprint in the order of the song.
        enum At<'p> {
            Part(usize, &'p PartSpan),
            Repeat(usize, Option<u32>),
        }
        let mut all: Vec<(usize, At)> = scan
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| (p.span.start, At::Part(i, p)))
            .collect();
        for (r, (span, _)) in scan.chorus_repeats.iter().enumerate() {
            all.push((
                span.start,
                At::Repeat(r, repeat_number(&text[span.clone()])),
            ));
        }
        all.sort_by_key(|(at, _)| *at);

        let mut parts = vec![String::new(); scan.parts.len()];
        let mut repeats = vec![String::new(); scan.chorus_repeats.len()];
        let mut seen: Vec<(String, Option<u32>)> = Vec::new();
        let mut verses = 0;
        let mut latest_chorus = None;
        let mut count = |key: (String, Option<u32>)| {
            seen.push(key.clone());
            seen.iter().filter(|k| **k == key).count()
        };
        let chorus = |n: usize, number: Option<u32>| match number {
            Some(number) => format!("the {} chorus R{number}", ordinal(n)),
            None => format!("the {} chorus", ordinal(n)),
        };
        for (_, at) in all {
            match at {
                At::Part(i, p) if p.kind == "verse" => {
                    verses += 1;
                    parts[i] = format!("verse {verses}");
                }
                At::Part(i, p) if p.kind == "chorus" => {
                    latest_chorus = Some(p.number);
                    parts[i] = chorus(count(("chorus".into(), p.number)), p.number);
                }
                At::Part(i, p) => {
                    let name = if p.kind == "verse*" {
                        "unnumbered verse".to_string()
                    } else {
                        part_name(&p.kind)
                    };
                    let n = count((name.clone(), None));
                    parts[i] = format!("the {} {name}", ordinal(n));
                }
                At::Repeat(r, number) => {
                    // `[]` or none: the latest chorus, whichever it was.
                    let number = number.or(latest_chorus.flatten());
                    repeats[r] = chorus(count(("chorus".into(), number)), number);
                }
            }
        }
        // A full reprint prints the latest chorus before it.
        let mut reprint_rows = std::collections::HashMap::new();
        for (span, _) in &scan.chorus_repeats {
            if text[span.clone()].starts_with("\\insertchorus*") {
                continue;
            }
            let latest = scan
                .parts
                .iter()
                .rev()
                .find(|p| p.kind == "chorus" && p.span.end <= span.start);
            if let Some(chorus) = latest {
                let rows = text[chorus.body.clone()]
                    .lines()
                    .filter(|l| !code_of(l).trim().is_empty())
                    .count();
                reprint_rows.insert(text[..span.start].matches('\n').count() + 1, rows);
            }
        }
        let mut rows_before = vec![0usize];
        for (n, line) in text.lines().enumerate() {
            let code = code_of(line).trim();
            let rows = match reprint_rows.get(&(n + 1)) {
                Some(&rows) => rows,
                // A part's opening and closing lines print nothing.
                None => usize::from(
                    !code.is_empty() && !code.starts_with("\\begin") && !code.starts_with("\\end"),
                ),
            };
            rows_before.push(rows_before.last().unwrap() + rows);
        }
        Names {
            text,
            parts,
            repeats,
            rows_before,
        }
    }
}

/// The `n` of `\insertchorus[n]` or `\insertchorus*[n]`, if given.
fn repeat_number(written: &str) -> Option<u32> {
    let rest = written.strip_prefix("\\insertchorus")?;
    let rest = rest.strip_prefix('*').unwrap_or(rest).strip_prefix('[')?;
    rest[..rest.find(']')?].trim().parse().ok()
}

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// A part's kind in words. A kind a song defines for itself
/// (`\newSongPartType{bridgeb}{**}…`) has no name but its macro.
fn part_name(kind: &str) -> String {
    let kind = kind.trim_end_matches('*');
    match kind {
        "verse" | "chorus" | "bridge" | "intro" | "outro" | "interlude" => kind.to_string(),
        "prechorus" => "pre-chorus".to_string(),
        "instrumental" => "instrumental part".to_string(),
        _ => format!("\\begin{kind} part"),
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
        // After joining the lines in pairs, which keeps every chord.
        assert_eq!(ids[..2], ["joinlines@5", "chordsoff@10"]);
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

    fn grade_of(text: &str, id: &str) -> &'static str {
        proposals(text)
            .into_iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("{id} in {:?}", ids(text)))
            .grade()
    }

    #[test]
    fn leaving_a_part_without_a_chorded_neighbour_is_the_last_resort() {
        // Four verses on one progression, the 2nd and 3rd plain: the 4th
        // may go plain too, chords on the first alone, but only as the
        // last resort (the user), on every paper and on small paper.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse\\chordsoff", &W),
                part("\\beginverse\\chordsoff", &V),
                part("\\beginverse", &W),
            ]
            .concat(),
        );
        assert_eq!(grade_of(&text, "chordsoff@20"), "last resort");
        assert_eq!(grade_of(&text, "chordsoffonsmallpage@20"), "last resort");
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.id == "chordsoff@20")
            .unwrap();
        assert!(
            p.description.ends_with("the last resort"),
            "{}",
            p.description
        );
        // Above a smaller type, which is the very last.
        let order: Vec<String> = ids(&text);
        let at = |id: &str| order.iter().position(|i| i == id).unwrap();
        assert!(at("chordsoffonsmallpage@20") < at("smalleronsmallpage@5"));

        // Chosen together, they are applied together.
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
        assert!(apply(&text, &two).is_ok());
    }

    #[test]
    fn stranding_a_neighbour_already_plain_is_the_last_resort() {
        // Verse 3 is plain and leans on verse 2; hiding verse 2's chords
        // leaves verse 3 without a chorded neighbour.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginverse\\chordsoff", &V),
            ]
            .concat(),
        );
        assert_eq!(grade_of(&text, "chordsoff@10"), "last resort");
    }

    #[test]
    fn a_song_already_breaking_the_rule_can_still_take_a_harmless_change() {
        // Verses 2 and 3 are plain side by side, with verse 1 next to 2 only:
        // verse 3 already breaks the rule. Hiding verse 5's chords, with
        // verse 4 chorded beside it, adds no break; hiding verse 5's
        // neighbour 4 would.
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
        assert_ne!(grade_of(&text, "chordsoff@30"), "last resort");
        assert_eq!(grade_of(&text, "chordsoff@25"), "last resort");
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
        // One chord changed is a slip, not another way to play it.
        let other_chords = ["\\[F]Refrén \\[G]zní", "\\[Am]a zní"];
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
    fn a_chorus_with_other_chords_keeps_them() {
        // The chorus sung again a key higher: its reminder would print the
        // words alone, and the player would play the first chorus's chords.
        let up = ["\\[G]Refrén \\[A]zní", "\\[D]a zní"];
        let text = song(&[part("\\beginchorus", &R), part("\\beginchorus", &up)].concat());
        assert!(!ids(&text).iter().any(|id| id.starts_with("insertchorus")));
    }

    #[test]
    fn apostrophes_and_punctuation_do_not_make_new_words() {
        let typed = ["\\[F]Tomorrow I'll \\[G]still be waiting,", "\\[C]a zní"];
        let retyped = [
            "\\[F]Tomorrow Ill \\[G]still be waiting",
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
        assert_eq!(
            kinds[..4],
            ["joinlines", "joinlines", "insertchorus*", "chordsoff"]
        );
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
            .zip(scan.parts.iter().filter(|p| p.kind == "verse"))
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

        // Then the second costs the plain third its chords right above
        // it: it reads verse 1's, two lines further.
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
        assert_eq!(p.score, CHORDS_OFF - 2 * PER_LINE_AWAY);
    }

    #[test]
    fn a_change_for_every_paper_counts_the_distance_on_small_paper() {
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
        // On A4 verse 3 reads verse 2's chords right above it; on A5, where
        // verse 2 has none, verse 1's, two lines further.
        assert_eq!(
            p.description,
            "verse 3 repeats the chords of verse 2: \\chordsoff"
        );
        assert_eq!(p.score, CHORDS_OFF - 2 * PER_LINE_AWAY);
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
    fn proposals_name_the_parts_as_the_page_prints_them() {
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginverse*", &["\\[C]Tag"]),
                part("\\beginverse", &W),
                "\\insertchorus\n\n".to_string(),
                part("\\beginchorus", &R),
            ]
            .concat(),
        );
        let said: Vec<String> = proposals(&text)
            .into_iter()
            .filter(|p| p.stage == Stage::AllPapers)
            .map(|p| p.description)
            .collect();
        assert!(
            said.iter()
                .any(|d| d.starts_with("verse 2 repeats the chords of verse 1 (1 line away)")),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|d| d.starts_with("the 2nd chorus is reprinted in full")),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|d| d.starts_with("the 3rd chorus has the words of the 1st chorus")),
            "{said:?}"
        );
    }

    fn join_of(text: &str, kind: &str) -> Option<Proposal> {
        proposals(text)
            .into_iter()
            .find(|p| p.kind == "joinlines" && p.description.contains(kind))
    }

    #[test]
    fn lines_of_every_part_of_a_kind_join_in_pairs() {
        let text = song(&[part("\\beginverse", &V), part("\\beginverse", &W)].concat());
        let p = join_of(&text, "verse").unwrap();
        assert_eq!(p.grade(), "good");
        assert!(p.needs_width_check());
        // Both halves have chords: a lyric row and a chord row go per pair.
        assert_eq!(p.gain_lines, 4);
        let out = apply(&text, &[&p]).unwrap();
        assert!(
            out.contains("\\beginverse\n    \\[C]Jedna \\[G]dvě \\[Am]tři \\[F]čtyři\n\\endverse"),
            "{out}"
        );
        assert!(
            out.contains("    \\[C]Pět \\[G]šest \\[Am]sedm \\[F]osm\n"),
            "{out}"
        );
        // It lengthens the verses' lines and no others.
        assert!(p.lengthens(Some(6), false));
        assert!(!p.lengthens(Some(3), false));
        assert!(!p.lengthens(None, true));
    }

    #[test]
    fn an_odd_number_of_lines_keeps_the_last_alone_when_every_part_has_it() {
        // Two half phrases and a whole one, in every verse alike.
        let a = [
            "\\[C]I walk along the river",
            "Down to the old \\[C]mill",
            "Waiting for the \\[Dm]morning train to come",
        ];
        let b = [
            "\\[C]A quiet autumn evening",
            "Shadows on the \\[C]hill",
            "Waiting for the \\[Dm]morning train to come",
        ];
        let text = song(&[part("\\beginverse", &a), part("\\beginverse", &b)].concat());
        let p = join_of(&text, "verse").unwrap();
        assert!(
            p.description
                .contains("the last of its 3 lines staying alone (4 lines into 2)"),
            "{}",
            p.description
        );
        let out = apply(&text, &[&p]).unwrap();
        assert!(
            out.contains("    \\[C]I walk along the river down to the old \\[C]mill\n    Waiting for the \\[Dm]morning train to come\n"),
            "{out}"
        );
        assert!(unjoinable(&text).is_empty());

        // Parts of 3 and of 5 lines would come out unlike: none, and why.
        let five = [a[0], a[1], a[2], b[0], b[1]];
        let text = song(&[part("\\beginverse", &a), part("\\beginverse", &five)].concat());
        assert!(join_of(&text, "verse").is_none());
        assert_eq!(
            unjoinable(&text),
            [
                "verse: the parts have 3 and 5 lines; an odd number pairs only when every part has the same"
            ]
        );
        // A short last line, a hook sung again, would stand out.
        let hook = [
            "\\[C]All the days I waited by the window",
            "Counting every car that passed the gate",
            "\\[F]Come home",
        ];
        let text = song(&part("\\beginchorus", &hook));
        assert!(join_of(&text, "chorus").is_none());
        assert_eq!(
            unjoinable(&text),
            [
                "chorus: the last of its 3 lines is much shorter than two joined, so the lines would come out unequal"
            ]
        );
        // A part of one line has nothing to pair.
        let text = song(&part("\\beginverse", &[a[0]]));
        assert_eq!(unjoinable(&text), ["verse: one line, nothing to pair"]);
    }

    #[test]
    fn a_capital_that_goes_on_a_sentence_is_made_lowercase() {
        let english = |text: String| text.replace("lang={czech}", "lang={english}");
        let a = [
            "\\[C]We walked along the river",
            "Down to the \\[G]mill",
            "\\[C]Then we saw the lights.",
            "Mary was \\[G]there",
        ];
        let b = [
            "\\[C]I knew the way to Mary",
            "Waiting by the \\[G]gate",
            "\\[C]She was going down",
            "I was \\[G]late",
        ];
        let text = english(song(
            &[part("\\beginverse", &a), part("\\beginverse", &b)].concat(),
        ));
        let p = join_of(&text, "verse").unwrap();
        let out = apply(&text, &[&p]).unwrap();
        // "down" is written lowercase elsewhere: sure.
        assert!(out.contains("river down to the"), "{out}");
        // After a sentence's end, and a name, and "I": as they are.
        assert!(out.contains("lights. Mary was"), "{out}");
        assert!(out.contains("down I was"), "{out}");
        // Nowhere else lowercase: made so, but named.
        assert!(out.contains("Mary waiting by"), "{out}");
        assert!(
            p.description.contains(
                "; 2 line starts made lowercase, Waiting though the song nowhere else writes it so: check it is not a name"
            ),
            "{}",
            p.description
        );
        // A quotation starts with its capital; German capitalises nouns.
        let quoted = ["\\[C]And then he said", "\\enquote{Go \\[G]home}"];
        let text = english(song(&part("\\beginverse", &quoted)));
        let out = apply(&text, &[&join_of(&text, "verse").unwrap()]).unwrap();
        assert!(out.contains("said \\enquote{Go"), "{out}");
        let text = song(&part("\\beginverse", &a)).replace("lang={czech}", "lang={german}");
        let out = apply(&text, &[&join_of(&text, "verse").unwrap()]).unwrap();
        assert!(out.contains("river Down"), "{out}");
    }

    #[test]
    fn why_a_kind_is_not_joined_is_said() {
        let commented = ["\\[C]Jedna % dvě", "\\[Am]tři"];
        let text = song(&part("\\beginverse", &commented));
        assert_eq!(
            unjoinable(&text),
            ["verse: line 6 ends in a comment, which would swallow the line joined after it"]
        );
    }

    fn chordlines(text: &str) -> Vec<Proposal> {
        proposals(text)
            .into_iter()
            .filter(|p| p.kind == "chordlines")
            .collect()
    }

    #[test]
    fn a_chord_after_the_last_word_goes_where_the_other_parts_have_it() {
        let v = ["\\[C]Jedna \\[G]dvě", "\\[Am]tři čtyři \\[F]", "\\[C]pět"];
        let w = [
            "\\[C]Šest \\[G]sedm",
            "\\[Am]osm devět",
            "\\[F]deset \\[C]jedenáct",
        ];
        let text = song(
            &[
                part("\\beginverse", &v),
                part("\\beginverse", &w),
                part("\\beginverse", &w),
            ]
            .concat(),
        );
        // Two of three break before F: the first verse goes their way.
        let found = chordlines(&text);
        assert_eq!(found.len(), 1, "{found:?}");
        let p = &found[0];
        assert_eq!(p.grade(), "correction");
        assert_eq!(
            p.description,
            "verse 1 splits its chords across the lines unlike verse 2: F from the end of line 7 to the start of line 8"
        );
        let out = apply(&text, &[p]).unwrap();
        assert!(
            out.contains("    \\[Am]tři čtyři\n    \\[F]\\[C]pět\n") == false,
            "{out}"
        );
        assert!(
            out.contains("    \\[Am]tři čtyři\n    \\[F] \\[C]pět\n"),
            "{out}"
        );
        assert!(chordlines(&out).is_empty());

        // The other way: a chord before the first word goes up.
        let text = song(
            &[
                part("\\beginverse", &v),
                part("\\beginverse", &v),
                part("\\beginverse", &w),
            ]
            .concat(),
        );
        let found = chordlines(&text);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].description.starts_with("verse 3 splits"),
            "{}",
            found[0].description
        );
        let out = apply(&text, &[&found[0]]).unwrap();
        assert!(
            out.contains("    \\[Am]osm devět \\[F]\n    deset \\[C]jedenáct\n"),
            "{out}"
        );

        // Marked right, it is not found again.
        let edit = dismissal(&text, &found[0]).unwrap();
        let mut marked = text.clone();
        marked.replace_range(edit.span.clone(), &edit.replacement);
        assert!(chordlines(&marked).is_empty());
    }

    #[test]
    fn a_chord_sung_inside_a_line_is_not_moved() {
        // F is sung on "čtyři" in one verse and on "deset" in the other:
        // a different melody, maybe, and no clean move either way.
        let v = ["\\[C]Jedna \\[G]dvě", "\\[Am]tři \\[F]čtyři", "\\[C]pět"];
        let w = [
            "\\[C]Šest \\[G]sedm",
            "\\[Am]osm devět",
            "\\[F]deset \\[C]jedenáct",
        ];
        let text = song(&[part("\\beginverse", &v), part("\\beginverse", &w)].concat());
        assert!(chordlines(&text).is_empty());
        // Different chords: nothing to compare.
        let x = [
            "\\[C]Šest \\[G]sedm",
            "\\[Am]osm devět",
            "\\[E]deset \\[C]jedenáct",
        ];
        let text = song(&[part("\\beginverse", &v), part("\\beginverse", &x)].concat());
        assert!(chordlines(&text).is_empty());
    }

    #[test]
    fn lines_join_only_where_every_part_of_the_kind_can() {
        let odd = ["\\[C]Jedna", "\\[G]dvě", "\\[Am]tři"];
        let text = song(&[part("\\beginverse", &V), part("\\beginverse", &odd)].concat());
        assert!(join_of(&text, "verse").is_none());

        // A comment would swallow the line joined after it; the comment of
        // a pair's second line stays at its end.
        let commented = ["\\[C]Jedna % dvě", "\\[Am]tři"];
        let text = song(&part("\\beginverse", &commented));
        assert!(join_of(&text, "verse").is_none());
        let commented = ["\\[C]Jedna", "\\[Am]tři % čtyři"];
        let text = song(&part("\\beginverse", &commented));
        let out = apply(&text, &[&join_of(&text, "verse").unwrap()]).unwrap();
        assert!(out.contains("\\[C]Jedna \\[Am]tři % čtyři\n"), "{out}");

        for line in ["\\rep{2}", "\\chordson", "\\textnote{x}", "\\iflargepaper"] {
            let lines = [&format!("\\[C]Jedna {line}") as &str, "\\[Am]tři"];
            let text = song(&part("\\beginverse", &lines));
            assert!(join_of(&text, "verse").is_none(), "{line}");
        }
        // A repeat sign at the end of the part stays at its end.
        let lines = ["\\[C]Jedna", "\\[Am]tři \\rep{2}"];
        assert!(join_of(&song(&part("\\beginverse", &lines)), "verse").is_some());

        // A gap between lines means something.
        let text = song("\\beginverse\n    \\[C]Jedna\n\n    \\[Am]tři\n\\endverse\n\n");
        assert!(join_of(&text, "verse").is_none());
    }

    #[test]
    fn of_two_verses_in_a_row_the_second_loses_its_chords() {
        // 1 2 R 3 4 R 5: verses 4 and 2, never 3 or 5, which the reader
        // meets coming back from the chorus.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                "\\insertchorus\n\n".to_string(),
                part("\\beginverse", &V),
            ]
            .concat(),
        );
        assert_eq!(plain_verses(text.clone(), 1), [4]);
        assert_eq!(plain_verses(text.clone(), 2), [2, 4]);
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.id == "chordsoff@20")
            .unwrap();
        // Verse 3 would read verse 2's chords across the chorus.
        assert_eq!(
            p.description,
            "verse 3 repeats the chords of verse 2 (2 lines away): \\chordsoff"
        );
        assert_eq!(p.grade(), "fair");
    }

    #[test]
    fn a_layout_in_a_corner_is_left_by_moving_the_chords() {
        // 1 2 R 3- 4 R 5: the verse after the chorus is plain. Better than
        // hiding one more is hiding 2 and 4 and showing 3 again.
        let text = song(
            &[
                part("\\beginverse", &V),
                part("\\beginverse", &W),
                part("\\beginchorus", &R),
                part("\\beginverse\\chordsoff", &V),
                part("\\beginverse", &W),
                "\\insertchorus\n\n".to_string(),
                part("\\beginverse", &V),
            ]
            .concat(),
        );
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.kind == "chordsoff")
            .unwrap();
        assert_eq!(p.id, "chordsoff@10+25-20");
        assert_eq!(
            p.description,
            "hide the chords of verse 2 and verse 4 and show them again in verse 3, so the verses without chords are nearer to theirs: \\chordsoff"
        );
        assert_eq!(p.grade(), "fair");
        // Each reads its chords right above it: verse 2 verse 1's, and
        // verse 4 verse 3's.
        let lines: Vec<usize> = p.sources.iter().map(|s| s.line).collect();
        assert_eq!(lines, [5, 20]);
        let out = apply(&text, &[&p]).unwrap();
        assert_eq!(plain_verses(out, 0), [2, 4]);
    }

    #[test]
    fn chords_go_where_they_are_nearest_and_stay_at_the_beginning() {
        let chorus = || part("\\beginchorus", &R);
        let again = || "\\insertchorus\n\n".to_string();
        let verse = || part("\\beginverse", &V);
        // V R V R V R: every verse is a chorus away from the one before, so
        // the last loses its chords.
        let text = song(&[verse(), chorus(), verse(), again(), verse(), again()].concat());
        assert_eq!(plain_verses(text, 1), [3]);
        // V V R V V R V V R: the second of the last pair, right below the
        // chords it repeats.
        let pairs: String = [verse(), verse(), chorus()]
            .into_iter()
            .chain((0..2).flat_map(|_| [verse(), verse(), again()]))
            .collect();
        assert_eq!(plain_verses(song(&pairs), 1), [6]);
        assert_eq!(plain_verses(song(&pairs), 3), [2, 4, 6]);
    }

    #[test]
    fn a_part_with_the_choruss_words_may_become_a_chorus() {
        // A modulated chorus came in as unnumbered verses.
        let up = ["\\[G]Refrén \\[A]zní", "\\[D]a zní"];
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginverse*", &up),
                part("\\beginverse*", &up),
            ]
            .concat(),
        );
        let found: Vec<Proposal> = proposals(&text)
            .into_iter()
            .filter(|p| p.kind == "restructure")
            .collect();
        // Each is a decision of its own.
        let ids: Vec<&str> = found.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["restructure@20", "restructure@15"]);
        let first = found.iter().find(|p| p.id == "restructure@15").unwrap();
        assert_eq!(
            first.description,
            "the 1st unnumbered verse has the words of the 1st chorus: if it is the chorus sung again, make it a chorus"
        );
        // It only becomes a chorus, with its chords; the copy after it is
        // then a chorus typed out again, which tidy reprints.
        let out = apply(&text, &[first]).unwrap();
        assert!(
            out.contains("\\beginchorus\n    \\[G]Refrén \\[A]zní\n    \\[D]a zní\n\\endchorus\n\n\\beginverse*\n"),
            "{out}"
        );
        let second = proposals(&out)
            .into_iter()
            .find(|p| p.id == "restructure@20")
            .unwrap();
        assert!(second.description.contains("the words of the 2nd chorus"));

        // Before any chorus: nothing to restructure.
        let text = song(&[part("\\beginverse*", &R), part("\\beginchorus", &R)].concat());
        assert!(!proposals(&text).iter().any(|p| p.kind == "restructure"));
    }

    #[test]
    fn rows_played_again_print_once_with_their_count() {
        let row = "| \\[Am]\\[] | \\[G]\\[] |";
        let text = song(&part(
            "\\begininstrumental",
            &[
                row,
                row,
                &format!("{row} \\rep{{2}}"),
                "| \\[C]\\[] | \\[F]\\[] |",
                row,
            ],
        ));
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.kind == "rep")
            .unwrap();
        assert_eq!(p.id, "rep@6");
        assert_eq!(p.gain_lines, 2);
        assert_eq!(p.grade(), "good");
        let out = apply(&text, &[&p]).unwrap();
        assert!(
            out.contains(&format!(
                "\\begininstrumental\n    {row} \\rep{{4}}\n    | \\[C]\\[] | \\[F]\\[] |\n    {row}\n\\endinstrumental"
            )),
            "{out}"
        );
    }

    #[test]
    fn rows_that_replay_chords_are_not_alike() {
        let row = "| ^\\[] | ^\\[] |";
        let text = song(&part("\\begininstrumental", &[row, row]));
        assert!(!proposals(&text).iter().any(|p| p.kind == "rep"));
        // Nor are sung lines: a repeated sung line is the singer's.
        let text = song(&part("\\beginverse", &["\\[C]La la", "\\[C]La la"]));
        assert!(!proposals(&text).iter().any(|p| p.kind == "rep"));
    }

    #[test]
    fn instrumental_rows_join_at_one_bar_line() {
        let text = song(&part(
            "\\begininstrumental",
            &["| \\[Am]\\[] | \\[G]\\[] |", "| \\[C]\\[] | \\[F]\\[] |"],
        ));
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.kind == "joinlines")
            .unwrap();
        assert_eq!(p.gain_lines, 1);
        let out = apply(&text, &[&p]).unwrap();
        assert!(
            out.contains("    | \\[Am]\\[] | \\[G]\\[] | \\[C]\\[] | \\[F]\\[] |\n"),
            "{out}"
        );
        // A row repeated on its own would come to repeat both.
        let text = song(&part(
            "\\begininstrumental",
            &["| \\[Am]\\[] |", "| \\[C]\\[] | \\rep{2}"],
        ));
        assert!(!proposals(&text).iter().any(|p| p.kind == "joinlines"));
    }

    #[test]
    fn a_part_cut_short_or_ending_on_no_chord_repeats_the_chords() {
        let short = ["\\[F]Refrén \\[G]zní"];
        let stop = ["\\[F]Refrén \\[G]zní", "\\[C]a zní \\[\\nochord]"];
        let other = ["\\[F]Refrén \\[G]zní", "\\[Am]a zní"];
        let text = song(
            &[
                part("\\beginchorus", &R),
                part("\\beginverse", &V),
                part("\\beginchorus", &short),
                part("\\beginverse", &V),
                part("\\beginchorus", &stop),
                part("\\beginverse", &V),
                part("\\beginchorus", &other),
            ]
            .concat(),
        );
        let ids = ids(&text);
        assert!(ids.contains(&"chordsoff@15".to_string()), "{ids:?}");
        assert!(ids.contains(&"chordsoff@24".to_string()), "{ids:?}");
        // A chord played otherwise is not one to read elsewhere.
        assert!(!ids.contains(&"chordsoff@34".to_string()), "{ids:?}");
    }

    fn czech_join(lines: &[&str]) -> (String, Proposal) {
        let text = song(&part("\\beginverse", lines));
        let p = proposals(&text)
            .into_iter()
            .find(|p| p.kind == "joinlines")
            .unwrap();
        (apply(&text, &[&p]).unwrap(), p)
    }

    #[test]
    fn joined_lines_take_the_comma_the_language_wants() {
        // Before "že": a comma, after the last word and before its chords.
        let (out, p) = czech_join(&["\\[C]Vím to \\[G]dobře\\[Am]", "\\[F]že se vrátíš"]);
        assert!(
            out.contains("    \\[C]Vím to \\[G]dobře,\\[Am] \\[F]že se vrátíš\n"),
            "{out}"
        );
        assert!(
            p.description.ends_with("; a comma at the end of line 6"),
            "{}",
            p.description
        );
        // Before "a": none.
        let (out, _) = czech_join(&["\\[C]Jdu domů", "a \\[G]zpívám"]);
        assert!(out.contains("    \\[C]Jdu domů a \\[G]zpívám\n"), "{out}");
        // Punctuation already there stays as it is.
        let (out, _) = czech_join(&["\\[C]Jdu domů,", "\\[G]zpívám"]);
        assert!(out.contains("    \\[C]Jdu domů, \\[G]zpívám\n"), "{out}");
        // Where no rule tells, the line is named.
        let (out, p) = czech_join(&["\\[C]Jdu domů", "\\[G]zpívám"]);
        assert!(out.contains("    \\[C]Jdu domů \\[G]zpívám\n"), "{out}");
        assert!(
            p.description
                .ends_with("; line 6 may want a comma at its end"),
            "{}",
            p.description
        );
    }

    #[test]
    fn a_finding_marked_right_is_not_found_again() {
        let text = song(
            &[
                part("\\beginverse", &["\\[C]Vím že se vrátíš", "a \\[G]zpívám"]),
                part("\\beginverse", &["\\[C]Vím že se vrátíš", "a \\[G]zpívám"]),
            ]
            .concat(),
        );
        let found: Vec<Proposal> = proposals(&text)
            .into_iter()
            .filter(|p| p.kind == "comma")
            .collect();
        assert_eq!(found.len(), 2);
        // The mark goes on a line of its own above, indented like the line.
        let edit = dismissal(&text, &found[0]).unwrap();
        let mut marked = text.clone();
        marked.replace_range(edit.span.clone(), &edit.replacement);
        assert!(
            marked.contains("\\beginverse\n    % nocheck\n    \\[C]Vím že"),
            "{marked}"
        );
        let left: Vec<String> = proposals(&marked)
            .into_iter()
            .filter(|p| p.kind == "comma")
            .map(|p| p.id)
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");
        // `% nocheck comma` anywhere: none of the kind.
        let all_marked = text.replacen("\\beginverse", "% nocheck comma\n\\beginverse", 1);
        assert!(!proposals(&all_marked).iter().any(|p| p.kind == "comma"));
        // A mark inside a part goes along above a joined line.
        let (out, _) = czech_join(&["\\[C]Jdu domů", "% nocheck", "a \\[G]zpívám"]);
        assert!(
            out.contains("    % nocheck\n    \\[C]Jdu domů a \\[G]zpívám\n"),
            "{out}"
        );
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
                ("joinlines", "good"),
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
