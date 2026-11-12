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
    // Spaces first: no rule after makes a run of them, and none of them
    // takes or adds a line, so the lines need no mapping back.
    let mut spaces = collapse_spaces(text);
    let spaced = apply(text, &spaces);
    let text = spaced.as_str();

    // A chord between two words gets its space before it, on the spaced
    // text; it adds no line either.
    let between = chords_between_words(text, &scan(text));
    let apart = apply(text, &between);
    let text = apart.as_str();
    spaces.extend(between);

    // Setup switches go onto the opening line next, so every rule after
    // finds them in one place.
    let mut setups = setup_to_opening_line(text, &scan(text));
    setups.sort_by_key(|c| c.span.start);
    let set_up = apply(text, &setups);
    let text = set_up.as_str();

    // A reminder typed by hand is the next thing to go: the chord moves
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

    // Moving a switch up or removing a reminder takes lines out; the later
    // changes are reported on the lines of the song as it was given.
    let mut later = moves;
    later.extend(choruses);
    {
        let original = text_before(&reminders);
        for change in &mut later {
            change.line = original(change.line);
        }
    }
    later.extend(reminders);
    {
        let original = text_before(&setups);
        for change in &mut later {
            change.line = original(change.line);
        }
    }
    // A part that became an `\insertchorus` took its moved setup with it:
    // the move is not news.
    let mut changes: Vec<Change> = spaces;
    changes.extend(setups.into_iter().filter(|setup| {
        !later
            .iter()
            .any(|c| c.line == setup.line && c.replacement.starts_with("\\insertchorus"))
    }));
    changes.extend(later);
    changes.sort_by_key(|c| c.line);
    (tidied, changes)
}

/// A chord written right after a word and before the space to the next one,
/// `dál\[G] a sál`, gets a space before it: `dál \[G] a sál` (the user).
/// Glued to the word, it belongs to neither: with `\chordsoff` the line
/// reads differently than with chords. Chords at the end of a line, `^` and
/// `\[]` slots, instrumentals and chords inside a word stay as they are.
fn chords_between_words(text: &str, scan: &super::Scan) -> Vec<Change> {
    let mut changes = Vec::new();
    for part in &scan.parts {
        if part.kind.starts_with("instrumental") {
            continue;
        }
        let mut at = part.body.start;
        for raw in text[part.body.clone()].split_inclusive('\n') {
            let code = code_of(raw);
            let line = text[..at].matches('\n').count() + 1;
            let mut search = 0;
            while let Some(found) = code[search..].find("\\[") {
                let start = search + found;
                // The whole group of chords written together.
                let mut end = start;
                while code[end..].starts_with("\\[") {
                    match code[end..].find(']') {
                        Some(close) => end += close + 1,
                        None => break,
                    }
                }
                search = end.max(start + 2);
                let before = code[..start].chars().next_back();
                let glued = before.is_some_and(|c| {
                    !c.is_whitespace() && !matches!(c, ']' | '{' | '~' | '|' | '^' | '(')
                });
                let group = &code[start..end];
                let rest = &code[end..];
                let spaced = rest.trim_start();
                let text_after = rest.starts_with(' ')
                    && !spaced.is_empty()
                    && (!spaced.starts_with('\\')
                        || spaced.starts_with("\\enquote")
                        || spaced.starts_with("\\multivoice"));
                if glued && text_after && !group.contains("\\[]") {
                    changes.push(Change {
                        span: at + start..at + start,
                        replacement: " ".to_string(),
                        line,
                        description: format!("{group} stands between two words: a space before it"),
                        removed_lines: 0,
                    });
                }
            }
            at += raw.len();
        }
    }
    changes
}

/// Runs of spaces become one space, and spaces ending a line go: TeX reads
/// a run of spaces as one and drops the ones at the end of a line, so the
/// song prints the same. Left as they are:
/// - the indentation, which shows the structure (and `latexindent` sets);
/// - comments, which may hold text lined up by hand (tablature, notes);
/// - the space after a lone backslash, `\ `, which is part of the command;
///   only the spaces after it go.
///
/// One change for the whole song, reported on the first line it touches.
fn collapse_spaces(text: &str) -> Vec<Change> {
    let mut out = String::with_capacity(text.len());
    let mut lines = Vec::new();
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        let body = raw.trim_end_matches(['\n', '\r']);
        let ending = &raw[body.len()..];
        let indent = body.len() - body.trim_start_matches([' ', '\t']).len();
        let code = code_of(body);
        let comment = &body[code.len()..];
        let mut line = String::from(&body[..indent]);
        let mut backslashes = 0;
        let mut in_run = false;
        for c in code[indent.min(code.len())..].chars() {
            if c == ' ' || c == '\t' {
                // A control space: the backslash takes this one.
                if !in_run && backslashes % 2 == 1 {
                    line.push(' ');
                    backslashes = 0;
                    in_run = true;
                    continue;
                }
                in_run = true;
                backslashes = 0;
                continue;
            }
            if in_run {
                if !line.ends_with(' ') || line.len() == indent {
                    line.push(' ');
                }
                in_run = false;
            }
            backslashes = if c == '\\' { backslashes + 1 } else { 0 };
            line.push(c);
        }
        // Before a comment one space stays; at the end of the line none.
        if in_run && !comment.is_empty() && !line.ends_with(' ') && line.len() > indent {
            line.push(' ');
        }
        line.push_str(comment);
        if line != body {
            lines.push(index + 1);
        }
        out.push_str(&line);
        out.push_str(ending);
    }
    let Some(&first) = lines.first() else {
        return Vec::new();
    };
    // One span, from the first line that changes to the end of the last.
    let start = text
        .split_inclusive('\n')
        .take(first - 1)
        .map(str::len)
        .sum();
    let end_in_text = text.len()
        - text
            .split_inclusive('\n')
            .skip(*lines.last().unwrap())
            .map(str::len)
            .sum::<usize>();
    let end_in_out = out.len() - (text.len() - end_in_text);
    vec![Change {
        span: start..end_in_text,
        replacement: out[start..end_in_out].to_string(),
        line: first,
        description: format!(
            "spaces collapsed on {} line{}",
            lines.len(),
            if lines.len() == 1 { "" } else { "s" }
        ),
        removed_lines: 0,
    }]
}

/// Maps a line of the song after `reminders` (or any changes that fold the
/// lines after their own into it) were applied back to the line it was on
/// before.
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
/// A copy that differs only in turning its chords off (`\chordsoff` or
/// `\chordsoffonsmallpage` after `\beginchorus`) becomes
/// `\insertchorus[][\chordsoff]`: the second bracket runs inside the
/// reprint, before the chorus's own switches. Those can only turn more
/// chords off, so a copy showing chords the first one hid stays typed out.
///
/// `\insertchorus` reprints the *latest* chorus, number and all, so a
/// chorus is only replaced when it matches the one before it (the last one
/// not replaced), with the same number. Whatever
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
    let mut latest: Option<&PartSpan> = None;
    for &repeat in &choruses {
        let Some((latest, setup)) =
            latest.and_then(|latest| Some((latest, reprint_setup(text, latest, repeat)?)))
        else {
            latest = Some(repeat);
            continue;
        };
        let number = repeat.number.map(|n| n.to_string()).unwrap_or_default();
        let replacement = match (&setup, repeat.number) {
            (None, Some(n)) => format!("\\insertchorus[{n}]"),
            (None, None) => "\\insertchorus".to_string(),
            (Some(setup), _) => format!("\\insertchorus[{number}][{setup}]"),
        };
        let description = match &setup {
            None => format!(
                "the chorus is the same as the one on line {}: \\insertchorus",
                latest.line
            ),
            Some(_) => format!(
                "the chorus is the one on line {} with its chords off: {replacement}",
                latest.line
            ),
        };
        changes.push(Change {
            span: repeat.span.clone(),
            replacement,
            line: repeat.line,
            description,
            removed_lines: text[repeat.span.clone()].matches('\n').count(),
        });
    }
    changes
}

/// Whether `repeat` prints what `latest` does, and if so, the setup a
/// reprint needs for it: `None` when it doesn't; `Some(None)` when it is
/// the same chorus, switches and all; `Some(Some(setup))` when the repeat
/// turns chords off that `latest` shows.
fn reprint_setup(text: &str, latest: &PartSpan, repeat: &PartSpan) -> Option<Option<String>> {
    let rest = |range: &Range<usize>| text[range.clone()].trim().to_string();
    let same_text = latest.number == repeat.number
        // Whatever stands behind `\endchorus` (`\rep{2}`) is not part of
        // what `\insertchorus` would print.
        && rest(&repeat.closing_rest).is_empty()
        && !uses_chord_memory(&text[latest.body.clone()])
        && !uses_chord_memory(&text[repeat.body.clone()])
        && lines(&text[latest.body.clone()]) == lines(&text[repeat.body.clone()]);
    if !same_text {
        return None;
    }
    let (was, now) = (rest(&latest.opening_rest), rest(&repeat.opening_rest));
    if was == now {
        return Some(None);
    }
    // Which chords each turns off: on large paper, on small paper.
    let off = |switches: &str| -> Option<(bool, bool)> {
        if !is_switch(switches) {
            return None;
        }
        let large = super::fit::has_word(switches, "chordsoff");
        Some((large, large || switches.contains("\\chordsoffonsmallpage")))
    };
    let (was_large, was_small) = off(&was)?;
    let (now_large, now_small) = off(&now)?;
    if (was_large && !now_large) || (was_small && !now_small) {
        return None;
    }
    Some(Some(now))
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
///   adds; a line typed without it would gain it), and shows no chords
///   (`\chordsoff` on the opening line or on a line of its own before the
///   words);
/// - nothing in it depends on the paper (`\iflargepaper`): the reminder
///   macro has one line for all;
/// - its words are the first line's of the chorus that is the latest at that
///   point, which may be an earlier hand-written reminder, since every
///   `\beginchorus` becomes the latest; one with words of its own stays.
///   Words as the reminder prints them: the main voice of a `\multivoice`,
///   the `\role` label apart. A reminder typed without the two-voice label
///   (`\role{Ty}{Arjen}`) the first line ends in becomes
///   `\insertchorus*[][\rolesoff]`, and so does a bare `\insertchorus*`
///   that reprinted it; one typed without a one-voice label stays;
/// - it has the latest chorus's number;
/// - no full `\insertchorus` reprints it before the next chorus: that would
///   go back to the chorus before;
/// - no chorus is conditional.
fn hand_written_reminders(text: &str, scan: &Scan) -> Vec<Change> {
    use super::fit::{chord_names, has_word};
    let choruses: Vec<&PartSpan> = scan.parts.iter().filter(|p| p.kind == "chorus").collect();
    if choruses.iter().any(|p| p.conditional) || scan.chorus_repeats.iter().any(|(_, c)| *c) {
        return Vec::new();
    }
    let code_lines = |part: &PartSpan| -> Vec<String> {
        text[part.body.clone()]
            .lines()
            .map(|l| code_of(l).trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };
    let sung = |part: &PartSpan| -> Vec<String> {
        code_lines(part)
            .into_iter()
            .filter(|l| !is_switch(l))
            .collect()
    };

    let mut changes = Vec::new();
    let mut latest: Option<&PartSpan> = None;
    for (k, &part) in choruses.iter().enumerate() {
        let lines = sung(part);
        // The switch may stand on the opening line or on a line of its own
        // before the words; one after them would be lost with the part.
        let body = code_lines(part);
        let switches_first = body.iter().take_while(|l| is_switch(l)).count() + 1 == body.len();
        let chords_off = has_word(code_of(&text[part.opening_rest.clone()]), "chordsoff")
            || body
                .iter()
                .any(|l| is_switch(l) && has_word(l, "chordsoff"));
        let next_chorus = choruses.get(k + 1).map_or(text.len(), |p| p.span.start);
        let reprinted = scan.chorus_repeats.iter().any(|(span, _)| {
            span.start >= part.span.end
                && span.start < next_chorus
                && !text[span.clone()].starts_with("\\insertchorus*")
        });
        let setup = latest.and_then(|latest| {
            let first = sung(latest).into_iter().next().unwrap_or_default();
            let fits = latest.number == part.number
                && lines.len() == 1
                && switches_first
                && ends_in_dots(&lines[0])
                && !lines[0].contains("\\if")
                && (chords_off || chord_names(&lines[0]).is_empty())
                && !reprinted
                && rest_is_empty(text, part);
            if !fits {
                return None;
            }
            let (words, role) = as_reminded(&first);
            let (typed, typed_role) = as_reminded(&lines[0]);
            if words.is_empty() || words != typed {
                return None;
            }
            match (role, typed_role) {
                (role, typed_role) if role == typed_role => Some(""),
                // Typed without a label for two voices: with the second
                // voice off the reminder would name only the first singer
                // of a line both sing, so the label is left out on purpose.
                // A one-voice label left out is likelier forgotten, and
                // putting it back changes the page: a person's call.
                (Some(role), None) if two_voice_role(&role) => Some("\\rolesoff"),
                _ => None,
            }
        });
        let Some(setup) = setup else {
            latest = Some(part);
            continue;
        };
        let number = part.number.map(|n| n.to_string()).unwrap_or_default();
        let star = |setup: &str| match (setup, part.number) {
            ("", Some(n)) => format!("\\insertchorus*[{n}]"),
            ("", None) => "\\insertchorus*".to_string(),
            (setup, _) => format!("\\insertchorus*[{number}][{setup}]"),
        };
        // An `\insertchorus*` after it reprinted this reminder, label
        // hidden; once it reprints the chorus instead, it needs the switch
        // too. One that brings its own setup is left to a person.
        let mut followers = Vec::new();
        if !setup.is_empty() {
            let mut own_setup = false;
            for (span, _) in &scan.chorus_repeats {
                if span.start < part.span.end || span.start >= next_chorus {
                    continue;
                }
                let written = &text[span.clone()];
                let brackets = written.trim_start_matches("\\insertchorus*");
                let its_number = brackets
                    .strip_prefix('[')
                    .and_then(|b| b.split(']').next())
                    .filter(|n| !n.is_empty());
                if its_number != part.number.map(|n| n.to_string()).as_deref() {
                    continue;
                }
                if brackets.matches('[').count() > 1 {
                    own_setup = true;
                }
                let line = text[..span.start].matches('\n').count() + 1;
                followers.push(Change {
                    span: span.clone(),
                    replacement: star(setup),
                    line,
                    description: format!(
                        "the reminder after it hides the label too: {}",
                        star(setup)
                    ),
                    removed_lines: 0,
                });
            }
            if own_setup {
                latest = Some(part);
                continue;
            }
        }
        let latest = latest.unwrap();
        changes.push(Change {
            span: part.span.clone(),
            replacement: star(setup),
            line: part.line,
            description: format!(
                "the chorus is a reminder of the one on line {}, typed out: {}",
                latest.line,
                star(setup)
            ),
            removed_lines: text[part.span.clone()].matches('\n').count(),
        });
        changes.extend(followers);
    }
    changes
}

/// What `\insertchorus*` prints of a line, as words to compare, and the
/// `\role` label it keeps at the end. The second voice is off in a
/// reminder, so `\multivoice{main}{second}` counts as its main voice.
fn as_reminded(line: &str) -> (String, Option<String>) {
    let mut main = String::new();
    let mut rest = line;
    while let Some(at) = rest.find("\\multivoice") {
        let after = &rest[at + "\\multivoice".len()..];
        main.push_str(&rest[..at]);
        // `\multivoiceoff` and the like are other commands.
        let groups = Some(after)
            .filter(|a| !a.starts_with(|c: char| c.is_ascii_alphabetic()))
            .and_then(|a| group(a.trim_start()))
            .and_then(|(first, a)| group(a.trim_start()).map(|(_, a)| (first, a)));
        match groups {
            Some((first, after)) => {
                main.push_str(first);
                rest = after;
            }
            None => {
                main.push_str("\\multivoice");
                rest = after;
            }
        }
    }
    main.push_str(rest);
    let role = main
        .match_indices("\\role")
        .find(|(at, _)| !main[at + 5..].starts_with(|c: char| c.is_ascii_alphabetic()))
        .map(|(at, _)| at);
    match role {
        Some(at) => (
            super::fit::lyric_words(&main[..at]),
            Some(main[at..].split_whitespace().collect::<Vec<_>>().join(" ")),
        ),
        None => (super::fit::lyric_words(&main), None),
    }
}

/// Whether a `\role{…}{…}` label names a second voice.
fn two_voice_role(role: &str) -> bool {
    role.strip_prefix("\\role")
        .and_then(|r| group(r.trim_start()))
        .and_then(|(_, r)| group(r.trim_start()))
        .is_some()
}

/// The inside of the `{…}` group `s` starts with, and what follows it.
fn group(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[1..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
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

/// The commands that set a part up for all of it, wherever they stand
/// before its first line: chord switches, chord memory, transposition, type
/// size, the second voice. (`\iflargepaper` opens lines of its own and
/// `\textnote` prints, so neither is one.)
const SETUP: [&str; 9] = [
    "chordsoff",
    "chordsoffonsmallpage",
    "memorize",
    "replay",
    "usechords",
    "copychords",
    "transpose",
    "smalleronsmallpage",
    "multivoiceoff",
];

/// Whether a line is made of setup commands alone, each with its `[…]` and
/// `{…}` arguments.
fn is_setup_line(code: &str) -> bool {
    let mut rest = code.trim();
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let Some(after) = rest.strip_prefix('\\') else {
            return false;
        };
        let name_len = after.bytes().take_while(u8::is_ascii_alphabetic).count();
        if !SETUP.contains(&&after[..name_len]) {
            return false;
        }
        rest = &after[name_len..];
        for (open, close) in [('[', ']'), ('{', '}')] {
            if let Some(inner) = rest.strip_prefix(open) {
                match inner.find(close) {
                    Some(end) if !inner[..end].contains(['[', '{']) => rest = &inner[end + 1..],
                    _ => return false,
                }
            }
        }
        rest = rest.trim_start();
    }
    true
}

/// Setup commands on lines of their own right after `\beginverse` (and the
/// other openings) join the opening line: `\beginverse\chordsoff`. That is
/// where most songs have them, and where the spec writes them; a switch on
/// a line of its own then means one in the middle of a part, which works
/// from there on. TeX reads both the same: a line end after a control word
/// is skipped like a space.
///
/// Only the lines between the opening line and the first one that is not
/// setup move, and none with a comment, nor onto an opening line that has
/// one.
fn setup_to_opening_line(text: &str, scan: &Scan) -> Vec<Change> {
    let mut changes = Vec::new();
    for part in &scan.parts {
        let opening = &text[part.opening_rest.clone()];
        if code_of(opening) != opening {
            continue;
        }
        let mut commands = String::new();
        let mut end = None;
        let mut at = part.body.start;
        let mut moved = 0;
        for raw in text[part.body.clone()].split_inclusive('\n') {
            let line = raw.trim_end_matches(['\n', '\r']);
            if code_of(line) != line || !is_setup_line(line) {
                break;
            }
            commands.push_str(line.trim());
            end = Some(at + line.len());
            at += raw.len();
            moved += 1;
        }
        let Some(end) = end else {
            continue;
        };
        let start = part.opening_rest.start + opening.trim_end().len();
        changes.push(Change {
            span: start..end,
            replacement: commands.clone(),
            line: part.line,
            description: format!("setup moved onto the opening line: {commands}"),
            removed_lines: moved,
        });
    }
    changes
}

/// Whether a line is a chord switch and nothing else (`\chordsoff` on a
/// line of its own inside the part): setup, not a sung line.
fn is_switch(code: &str) -> bool {
    code.replace("\\chordsoffonsmallpage", "")
        .replace("\\chordsoff", "")
        .trim()
        .is_empty()
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

    /// The patterns, and whether they are trusted for the start of a word's
    /// last syllable (Czech and Slovak ones; English ones miss syllables,
    /// "a-way").
    pub struct Hyphenator(Standard, bool);

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
            Some(Hyphenator(patterns, language != Language::EnglishUS))
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
                // Back over consonants to the syllable's start; where the
                // patterns find every syllable, over its vowel too: a chord
                // drawn on the consonants that close a syllable belongs to
                // the syllable ("schodec\[F]h" → "scho\[F]dech", the user).
                let vowels = word[start..*index]
                    .iter()
                    .copied()
                    .filter(|&c| is_vowel(c))
                    .count();
                // Only in the word's last syllable, with nothing but its
                // closing consonants after the chord: the patterns miss
                // syllables in Czech too ("u-mět"), and a chord with a vowel
                // after it may well open one they missed.
                let last_syllable = word[*index..].iter().all(|&c| !is_vowel(c));
                let closing = vowels > 0
                    && hyphenator.1
                    && last_syllable
                    && word[start..*index]
                        .iter()
                        .copied()
                        .skip_while(|&c| !is_vowel(c))
                        .skip_while(|&c| is_vowel(c))
                        .all(|c| !is_vowel(c));
                let start = if start == *index || (vowels > 0 && !closing) {
                    // It stays, but never between the two letters of "ch".
                    let splits_ch = *index > 0
                        && word[*index - 1].eq_ignore_ascii_case(&'c')
                        && word[*index].eq_ignore_ascii_case(&'h');
                    if !splits_ch {
                        continue;
                    }
                    *index - 1
                } else {
                    start
                };
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
        let (tidied, _) = tidy(&text);
        assert!(tidied.contains("\n\\insertchorus\n"), "{tidied}");

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

        for (first, second) in [("[1]", ""), ("", "[2]"), ("\\chordsoff", "")] {
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
    fn a_chord_between_two_words_gets_a_space_before_it() {
        let text = "\\beginsong{T}[\n]\n\\beginverse\n    Tak jdu \\[E]dál a dál\\[G] a sál \\[D]střídá sál,\n    I re\\[Bm]member\\[Em]\\[D] when\n    na konci\\[G] \\[D]\n    compa-\\[A]ny\n\\endverse\n\\begininstrumental\n    | \\[C]\\[] | ^\\[] |\n\\endinstrumental\n\\endsong\n";
        let (out, changes) = tidy(text);
        assert!(out.contains("dál a dál \\[G] a sál"), "{out}");
        assert!(out.contains("re\\[Bm]member \\[Em]\\[D] when"), "{out}");
        // At the end of a line, inside a word, in an instrumental: as is.
        assert!(out.contains("na konci\\[G] \\[D]\n"), "{out}");
        assert!(out.contains("compa-\\[A]ny\n"), "{out}");
        assert!(out.contains("| \\[C]\\[] | ^\\[] |"), "{out}");
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(tidy(&out).1.len(), 0);
    }

    #[test]
    fn runs_of_spaces_become_one() {
        let text = "\\beginverse\n    \\[C]Hello   \\[G]there,  \n\t\\[F]a~b \t c   % keep   this\n    \\lrep\\   x \\rrep\\ \n    %\\[A]a   b\n\\endverse\n";
        let (tidied, changes) = tidy(text);
        assert_eq!(
            tidied,
            "\\beginverse\n    \\[C]Hello \\[G]there,\n\t\\[F]a~b c % keep   this\n    \\lrep\\ x \\rrep\\ \n    %\\[A]a   b\n\\endverse\n"
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].line, 2);
        assert!(tidy(&tidied).1.is_empty());
        // `\\` is a backslash pair, not a control space.
        assert_eq!(tidy("x\\\\   y\n").0, "x\\\\ y\n");
    }

    #[test]
    fn a_copy_with_its_chords_off_becomes_insertchorus_with_the_switch() {
        let off = song(&[
            chorus("", BODY, ""),
            VERSE.into(),
            chorus("\\chordsoff", BODY, ""),
        ]);
        let (tidied, changes) = tidy(&off);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(
            tidied.contains("\n\\insertchorus[][\\chordsoff]\n"),
            "{tidied}"
        );
        assert!(tidy(&tidied).1.is_empty());

        let numbered = song(&[
            chorus("[2]", BODY, ""),
            chorus("[2]\\chordsoffonsmallpage", BODY, ""),
        ]);
        assert!(
            tidy(&numbered)
                .0
                .contains("\\insertchorus[2][\\chordsoffonsmallpage]")
        );

        // Hidden on small paper, then hidden on both: the reprint's own
        // \chordsoffonsmallpage adds nothing to \chordsoff.
        let more = song(&[
            chorus("\\chordsoffonsmallpage", BODY, ""),
            chorus("\\chordsoff", BODY, ""),
        ]);
        assert!(tidy(&more).0.contains("\\insertchorus[][\\chordsoff]"));

        // Every later copy is compared with the chorus, not with a reprint.
        let twice = song(&[
            chorus("", BODY, ""),
            chorus("\\chordsoff", BODY, ""),
            chorus("\\chordsoff", BODY, ""),
        ]);
        assert_eq!(
            tidy(&twice)
                .0
                .matches("\\insertchorus[][\\chordsoff]")
                .count(),
            2
        );

        // Chords the first one hides can't be brought back, and other
        // commands don't count as switches.
        for (first, second) in [
            ("\\chordsoff", "\\chordsoffonsmallpage"),
            ("", "\\chordsoff\\textnote{x}"),
        ] {
            let text = song(&[chorus(first, BODY, ""), chorus(second, BODY, "")]);
            assert!(tidy(&text).1.is_empty(), "{first} {second}");
        }
    }

    #[test]
    fn setup_on_lines_of_its_own_joins_the_opening_line() {
        let text = "\\beginverse\n    \\chordsoff\n    \\memorize[bridge]\n    \\[C]Hello\n    \\chordson\n    there\n\\endverse\n";
        let (tidied, changes) = tidy(text);
        assert_eq!(
            tidied,
            "\\beginverse\\chordsoff\\memorize[bridge]\n    \\[C]Hello\n    \\chordson\n    there\n\\endverse\n"
        );
        assert_eq!(changes.len(), 1);
        assert!(tidy(&tidied).1.is_empty());

        // Onto what is there already; a switch after the words stays.
        let text = "\\beginchorus[2]\\chordsoffonsmallpage \n    \\transpose{-2}\n    Hi\n    \\chordsoff\n    Ho\n\\endchorus\n";
        assert_eq!(
            tidy(text).0,
            "\\beginchorus[2]\\chordsoffonsmallpage\\transpose{-2}\n    Hi\n    \\chordsoff\n    Ho\n\\endchorus\n"
        );

        // A comment, a command that is not setup, or a conditional stays.
        for text in [
            "\\beginverse % why\n    \\chordsoff\n    Hi\n\\endverse\n",
            "\\beginverse\n    \\chordsoff % why\n    Hi\n\\endverse\n",
            "\\beginverse\n    \\textnote{x}\n    Hi\n\\endverse\n",
            "\\beginverse\n    \\iflargepaper\\chordsoff\\fi\n    Hi\n\\endverse\n",
            "\\beginverse\n    \\chordsoffx\n    Hi\n\\endverse\n",
        ] {
            assert!(tidy(text).1.is_empty(), "{text}");
        }
    }

    #[test]
    fn lines_are_reported_on_the_song_as_given() {
        // The setup line goes, so the chorus after it moves up a line in
        // the tidied song; it is still reported where it was.
        let text = song(&[
            "\\beginverse\n    \\chordsoff\n    \\[C]A verse\n\\endverse\n\n".into(),
            chorus("", BODY, ""),
            chorus("", BODY, ""),
        ]);
        let (_, changes) = tidy(&text);
        let chorus_line = text.lines().position(|l| l == "\\beginchorus").unwrap() + 1;
        let second = text
            .lines()
            .enumerate()
            .filter(|(_, l)| *l == "\\beginchorus")
            .nth(1)
            .unwrap()
            .0
            + 1;
        assert!(chorus_line < second);
        assert_eq!(
            changes.iter().map(|c| c.line).collect::<Vec<_>>(),
            vec![5, second]
        );
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
    fn a_chord_on_the_consonants_closing_a_syllable_moves_to_its_start() {
        // Czech patterns find every syllable: back over the vowel too.
        assert_eq!(tidied_line("czech", "po schodec\\[F]h"), "po scho\\[F]dech");
        assert_eq!(tidied_line("czech", "po schode\\[F]ch"), "po scho\\[F]dech");
        // English ones miss syllables ("a-way"): only "ch" is kept whole.
        assert_eq!(
            tidied_line("english", "so muc\\[C]h more"),
            "so mu\\[C]ch more"
        );
        assert_eq!(tidied_line("english", "awa\\[C]y"), "awa\\[C]y");
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
    fn a_reminder_with_chordsoff_on_a_line_of_its_own_becomes_insertchorus_star() {
        // Sea of Machines: the switch on its own line is not a sung line.
        let full = "    \\[E]I've seen the \\[B]future in a \\[F#m]dream \\[C#m] \\role{Prophet}\n    \\[A]I've seen a \\[E]sea\n";
        let typed = "    \\chordsoff\n    \\[E]I've seen the \\[B]future in a \\[F#m]dream... \\[C#m] \\role{Prophet}\n    %\\[A]I've seen a \\[E]sea\n";
        let text = song(&[chorus("", full, ""), VERSE.into(), chorus("", typed, "")]);
        let (tidied, changes) = tidy(&text);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(
            tidied,
            song(&[
                chorus("", full, ""),
                VERSE.into(),
                "\\insertchorus*\n\n".into()
            ])
        );
        // After the words, the switch would be lost with the part: it stays.
        let late = "    \\[E]I've seen the \\[B]future in a \\[F#m]dream...\n    \\chordsoff\n";
        let text = song(&[chorus("", full, ""), VERSE.into(), chorus("", late, "")]);
        assert!(
            tidy(&text)
                .1
                .iter()
                .all(|c| !c.description.contains("reminder"))
        );
    }

    #[test]
    fn a_reminder_of_the_main_voice_without_the_label_hides_the_label() {
        // Connect the Dots.
        let full = "    We all \\[C#m]know \\multivoice{}{We all} \\[E]{\\multivoice{}{know,}} \\role{Ty}{Arjen}\n    \\[D]We are dying\n";
        let typed = "    We all \\[C#m]know...\n";
        let text = song(&[
            chorus("", full, ""),
            VERSE.into(),
            chorus("\\chordsoff", typed, ""),
            VERSE.into(),
            "\\insertchorus*\n\n".into(),
        ]);
        let (tidied, changes) = tidy(&text);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert_eq!(
            tidied,
            song(&[
                chorus("", full, ""),
                VERSE.into(),
                "\\insertchorus*[][\\rolesoff]\n\n".into(),
                VERSE.into(),
                "\\insertchorus*[][\\rolesoff]\n\n".into(),
            ])
        );
        assert!(tidy(&tidied).1.is_empty());

        // With the label typed, nothing to hide; with another label, or the
        // second voice's words, it is a reminder of its own.
        let same = "    We all \\[C#m]know... \\role{Ty}{Arjen}\n";
        let text = song(&[chorus("", full, ""), chorus("\\chordsoff", same, "")]);
        assert!(tidy(&text).0.contains("\n\\insertchorus*\n"));
        // A one-voice label left out may be a slip: left to a person.
        let one = "    We all \\[C#m]know \\role{Ty}\n    \\[D]We are dying\n";
        let text = song(&[chorus("", one, ""), chorus("\\chordsoff", typed, "")]);
        assert!(tidy(&text).1.is_empty());
        for other in [
            "    We all know... \\role{Arjen}\n",
            "    We all know We all...\n",
        ] {
            let text = song(&[chorus("", full, ""), chorus("\\chordsoff", other, "")]);
            assert!(tidy(&text).1.is_empty(), "{other}");
        }
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
