use super::SongPrinter;
use crate::models::{Chord, Line, Part, PartType, Song};
use crate::parsers::text::{is_unsung, tokenize};
use std::io::Write;

pub struct LatexPrinter;

impl SongPrinter for LatexPrinter {
    fn print(&self, song: &Song, writer: &mut dyn Write) -> std::io::Result<()> {
        // 1. Begin Song & Metadata Block
        writeln!(writer, "\\beginsong{{{}}}[", escape(&song.title))?;
        writeln!(writer, "    by={{{}}},", escape(&song.artist))?;

        match &song.language {
            Some(lang) => writeln!(writer, "    lang={{{}}},", escape(lang))?,
            None => writeln!(writer, "    % lang={{czech|english}},")?,
        }
        match &song.tempo {
            Some(tempo) => writeln!(writer, "    tem={{{}}},", escape(tempo))?,
            None => writeln!(writer, "    % tem={{}},")?,
        }
        match &song.meter {
            Some(meter) => writeln!(writer, "    met={{{}}},", escape(meter))?,
            None => writeln!(writer, "    % met={{}},")?,
        }
        match &song.tuning {
            Some(tuning) => writeln!(writer, "    tun={{{}}},", escape(tuning))?,
            None => writeln!(writer, "    tun={{Standard}},")?,
        }
        match &song.key {
            Some(key) => writeln!(writer, "    key={{{}}},", escape_key(key))?,
            None => writeln!(writer, "    % key={{}},")?,
        }
        match &song.capo {
            None | Some(0) => writeln!(writer, "    % cap={{}},")?,
            Some(capo) => writeln!(writer, "    cap={{{}}},", capo)?,
        }
        writeln!(writer, "    src={{{}}},", escape(&song.source_website))?;
        // Metadata the song format has no key for is kept as a comment.
        for (label, value) in &song.extra {
            writeln!(
                writer,
                "    % {}={{{}}},",
                one_line(&label.to_lowercase()),
                one_line(&escape(value))
            )?;
        }
        writeln!(writer, "]")?;
        writeln!(writer)?;

        // 2. Iterate through parts
        for part in &song.parts {
            // A part of chords alone is played, not sung, so it is set as the
            // instrumental it is, in bars, whatever the site called it: an
            // intro, an interlude, a solo or an outro. Tablature is the
            // exception; it is not chords.
            if is_chords_only(part) {
                writeln!(writer, "\\begininstrumental")?;
                for row in instrumental_rows(part) {
                    writeln!(writer, "{INDENT}{row}")?;
                }
                writeln!(writer, "\\endinstrumental")?;
                writeln!(writer)?;
                continue;
            }

            // Open the correct environment based on the part type
            let env_close_str = match &part.part_type {
                PartType::Intro => {
                    writeln!(writer, "\\beginintro")?;
                    "\\endintro"
                }
                PartType::Verse => {
                    writeln!(writer, "\\beginverse")?;
                    "\\endverse"
                }
                PartType::PreChorus => {
                    writeln!(writer, "\\beginprechorus")?;
                    "\\endprechorus"
                }
                PartType::Chorus => {
                    writeln!(writer, "\\beginchorus")?;
                    "\\endchorus"
                }
                PartType::Bridge => {
                    writeln!(writer, "\\beginbridge")?;
                    "\\endbridge"
                }
                PartType::Instrumental => {
                    writeln!(writer, "\\begininstrumental")?;
                    "\\endinstrumental"
                }
                PartType::Tab => {
                    writeln!(writer, "\\beginverse*")?;
                    writeln!(writer, "{INDENT}% [TAB]")?;
                    "\\endverse*"
                }
                PartType::Outro => {
                    writeln!(writer, "\\beginoutro")?;
                    "\\endoutro"
                }
                PartType::Other(name) => {
                    writeln!(writer, "\\beginverse*")?;
                    writeln!(writer, "{INDENT}% [{}]", one_line(&name.to_uppercase()))?;
                    "\\endverse*"
                }
            };

            // Merge the chords into the lyrics, line by line. Whether a
            // quotation mark opens or closes follows from the ones before it,
            // so the whole part is resolved before the first line is written.
            let marks = quote_marks(part);
            for (line, marks) in part.lines.iter().zip(&marks) {
                // TeX skips the spaces a line starts with, so a margin the
                // source left in front of the first chord is dropped.
                let rendered = render_line(line, marks);
                match rendered.trim() {
                    "" => writeln!(writer)?,
                    text => writeln!(writer, "{INDENT}{text}")?,
                }
            }

            // Close the environment
            writeln!(writer, "{}", env_close_str)?;
            writeln!(writer)?;
        }

        writeln!(writer, "\\endsong")?;

        Ok(())
    }

    fn extension(&self) -> &str {
        "tex"
    }
}

/// What part content is indented by, as the songbook's `latexindent.yaml` does
/// it, so running latexindent over a draft changes nothing. The one exception
/// is a quotation running over two lines, whose second line latexindent
/// indents a step further. The indent is rendering-neutral either way.
const INDENT: &str = "    ";

/// Whether a part is played rather than sung: it holds chords, and nothing in
/// it is a lyric. Tablature is set as it is, never as an instrumental.
fn is_chords_only(part: &Part) -> bool {
    !matches!(part.part_type, PartType::Tab)
        && part.lines.iter().any(|line| !line.chords.is_empty())
        && part.lines.iter().all(is_unsung)
}

/// The rows of an instrumental, each a run of bars of fixed-width slots:
/// `| \[Am]\[] | \[F]\[C] |`.
///
/// The source's own bar lines are kept where it drew them. A row drawn
/// without any gets none: its chords are written as slots one after the
/// other (`\[Em]\[G]\[C]\[D]`). How many of them make a bar is not on the
/// page, and a guess would be wrong as often as right (Letter to Dana), so
/// the bar lines are left to the hand edit. A bar
/// has two slots, the songbook's common choice, or as many as it has chords;
/// `\[]` is the slot nothing new is played in. A repeat count written on the
/// row (`x2`, `2x`, `(3×)`) becomes `\rep{n}` behind it. The other marks a
/// sheet decorates a chord line with carry no rhythm the slots could keep, so
/// they are dropped.
fn instrumental_rows(part: &Part) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for line in &part.lines {
        let mut tokens: Vec<(usize, BarToken)> = line
            .chords
            .iter()
            .map(|placed| (placed.position, BarToken::Chord(chord_name(&placed.chord))))
            .collect();
        for (column, token) in tokenize(&line.lyrics) {
            if let Some(count) = repeat_count(token) {
                tokens.push((column, BarToken::Repeat(count)));
            } else if token.chars().all(|c| c == '|' || c == ':') {
                tokens.push((column, BarToken::BarLine));
            } else if token == "%" {
                // "As the bar before": a slot of its own, like a chord.
                tokens.push((column, BarToken::Chord("\\%".to_string())));
            }
        }
        tokens.sort_by_key(|(column, _)| *column);

        let drawn_bars = tokens.iter().any(|(_, t)| matches!(t, BarToken::BarLine));
        let mut bars: Vec<Vec<String>> = Vec::new();
        let mut current: Vec<String> = Vec::new();
        let mut repeat = None;
        // Whether a bar line has been drawn yet: a bar between two of them
        // is a bar even when nothing new is played in it, so the row keeps
        // its length.
        let mut opened = false;
        for (_, token) in tokens {
            match token {
                BarToken::Chord(name) if drawn_bars => current.push(name),
                BarToken::Chord(name) => bars.push(vec![name]),
                BarToken::BarLine => {
                    if opened || !current.is_empty() {
                        bars.push(std::mem::take(&mut current));
                    }
                    opened = true;
                }
                BarToken::Repeat(count) => repeat = Some(count),
            }
        }
        if !current.is_empty() {
            bars.push(current);
        }

        if bars.is_empty() {
            // A row with nothing but a repeat count repeats the row above it,
            // unless that row already says how often it is played.
            if let (Some(count), Some(last)) = (repeat, rows.last_mut())
                && !last.contains("\\rep{")
            {
                last.push_str(&format!(" \\rep{{{count}}}"));
            }
            continue;
        }

        if !drawn_bars {
            let mut row: String = bars
                .iter()
                .flatten()
                .map(|name| format!("\\[{name}]"))
                .collect();
            if let Some(count) = repeat {
                row.push_str(&format!(" \\rep{{{count}}}"));
            }
            rows.push(row);
            continue;
        }

        let mut row = String::from("|");
        for bar in &bars {
            row.push(' ');
            for name in bar {
                row.push_str(&format!("\\[{name}]"));
            }
            for _ in bar.len()..SLOTS_PER_BAR {
                row.push_str("\\[]");
            }
            row.push_str(" |");
        }
        if let Some(count) = repeat {
            row.push_str(&format!(" \\rep{{{count}}}"));
        }
        rows.push(row);
    }
    rows
}

/// The slots a bar of an instrumental has at the least.
const SLOTS_PER_BAR: usize = 2;

/// What an instrumental row is read into, in the order it was written.
enum BarToken {
    Chord(String),
    BarLine,
    Repeat(u32),
}

/// The count of a repeat mark: `x2`, `2x`, `3×`, in brackets or not.
fn repeat_count(token: &str) -> Option<u32> {
    let lower = token.trim_matches(['(', ')', '[', ']']).to_lowercase();
    let digits = lower.trim_start_matches('x').trim_end_matches(['x', '×']);
    if digits.len() == lower.len() || digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// How a `"` of the lyrics reaches LaTeX: csquotes' `\enquote{…}`, opened by
/// the first mark of a quotation and closed by the one that answers it. A mark
/// nothing answers is printed as the character it is: an `\enquote{` that is
/// never closed would swallow the end of the part and stop the build.
#[derive(Clone, Copy, Debug, PartialEq)]
enum QuoteMark {
    Open,
    Close,
    /// Babel's Czech makes `"` an active shorthand, so not the bare character.
    Stray,
}

impl QuoteMark {
    fn latex(self) -> &'static str {
        match self {
            QuoteMark::Open => "\\enquote{",
            QuoteMark::Close => "}",
            QuoteMark::Stray => "\\textquotedbl{}",
        }
    }
}

/// Resolves every `"` of a part, line by line and in reading order. A
/// quotation runs from each odd mark to the one after it, wherever that one
/// sits. Pairing stops at the part, so a stray mark cannot carry over into the
/// rest of the song. When the part holds an odd number of marks, the last one
/// is the one left without a partner, and it stays a plain mark.
fn quote_marks(part: &Part) -> Vec<Vec<QuoteMark>> {
    let total: usize = part
        .lines
        .iter()
        .map(|l| l.lyrics.matches('"').count())
        .sum();
    let mut seen = 0;
    part.lines
        .iter()
        .map(|line| {
            line.lyrics
                .matches('"')
                .map(|_| {
                    seen += 1;
                    if seen == total && total % 2 == 1 {
                        QuoteMark::Stray
                    } else if seen % 2 == 1 {
                        QuoteMark::Open
                    } else {
                        QuoteMark::Close
                    }
                })
                .collect()
        })
        .collect()
}

/// The character a chord attaches to. A chord is sung on the word that follows
/// it, never on a quotation mark, so when nothing but spaces and marks stand
/// between the two the chord moves along to the word — past the `\enquote{`,
/// which may not be broken into, or past the `}` that ends the quotation.
/// Everything else, a chord sitting on a space above all, stays where it is.
fn chord_target(chars: &[char], position: usize) -> usize {
    let mut target = position;
    let mut after_last_mark = None;
    while target < chars.len() {
        match chars[target] {
            '"' => {
                target += 1;
                after_last_mark = Some(target);
            }
            c if c.is_whitespace() => target += 1,
            _ => break,
        }
    }

    match after_last_mark {
        // No mark in the way: the chord belongs where the source put it.
        None => position,
        // The line runs out behind the quotation mark — the chord has no word
        // left to attach to and only needs to clear the mark itself.
        Some(after_last_mark) if target == chars.len() => after_last_mark,
        Some(_) => target,
    }
}

/// Writes one lyric character out, the quotation marks as resolved for them.
fn push_lyric(c: char, marks: &[QuoteMark], next_mark: &mut usize, out: &mut String) {
    if c == '"' {
        if let Some(mark) = marks.get(*next_mark) {
            out.push_str(mark.latex());
        }
        *next_mark += 1;
    } else {
        escape_char(c, out);
    }
}

/// Merges a line's chords into its lyrics.
fn render_line(line: &Line, marks: &[QuoteMark]) -> String {
    let chars: Vec<char> = line.lyrics.chars().collect();
    // Where the words end. Chords behind it are written one space apart,
    // whatever run of spaces the source spread them over.
    let end = chars.len() - chars.iter().rev().take_while(|c| c.is_whitespace()).count();
    let mut out = String::with_capacity(line.lyrics.len());
    let mut current_pos = 0;
    let mut next_mark = 0;

    // Ensure chords are processed left-to-right
    let mut sorted_chords = line.chords.clone();
    sorted_chords.sort_by_key(|c| c.position);

    for placed in &sorted_chords {
        let chord_str = chord_name(&placed.chord);

        // Catch the lyrics cursor up to the word this chord is sung on.
        let target = chord_target(&chars, placed.position);
        if target > end {
            while current_pos < end {
                push_lyric(chars[current_pos], marks, &mut next_mark, &mut out);
                current_pos += 1;
            }
            out.push(' ');
            out.push_str(&format!("\\[{}]", chord_str));
            continue;
        }
        while current_pos < target {
            if current_pos < chars.len() {
                push_lyric(chars[current_pos], marks, &mut next_mark, &mut out);
            } else {
                // Pad with spaces if chord extends past existing lyrics
                out.push(' ');
            }
            current_pos += 1;
        }

        // --- SPACING RULE CHECK ---
        // If the chord lands exactly on a space immediately following a non-space (word),
        // inject an extra space before placing the chord. A chord that moved along to a
        // word already has that space — and the quotation mark — written out in front of it.
        if target == placed.position
            && placed.position > 0
            && placed.position < chars.len()
            && !chars[placed.position - 1].is_whitespace()
            && chars[placed.position].is_whitespace()
        {
            out.push(' ');
        }

        // Inject the inline LaTeX chord
        out.push_str(&format!("\\[{}]", chord_str));
    }

    // Push any remaining lyrics characters after the last chord
    while current_pos < end {
        push_lyric(chars[current_pos], marks, &mut next_mark, &mut out);
        current_pos += 1;
    }

    out
}

/// Writes a chord out as the `songs` package reads it: root, quality and bass
/// note, the flats of each rewritten for it.
///
/// The "no chord" mark is not a chord to be named but the absence of one, and
/// the package has a macro that sets it — which goes where a name would, the
/// mark being played in its column like any chord.
fn chord_name(chord: &Chord) -> String {
    if chord.is_no_chord() {
        return "\\nochord".to_string();
    }

    let mut name = flat_note(&chord.root);
    if let Some(modifier) = &chord.modifier {
        name.push_str(&flat_degrees(modifier));
    }
    if let Some(bass) = &chord.bass {
        name.push('/');
        name.push_str(&flat_note(bass));
    }
    chord_safe(name)
}

/// Keeps only what a chord name is made of. A chord reaches LaTeX unescaped,
/// inside `\[...]` — and its text comes from the web page — so a page could
/// otherwise end the chord with `]` and follow it with any TeX it likes, which a
/// build with `-shell-escape` would run (`\directlua{os.execute(...)}`).
/// Letters, digits, the accidentals and the punctuation of chord qualities
/// pass, as does anything beyond ASCII (`°`, `ø`, `Δ`); markup does not.
fn chord_safe(name: String) -> String {
    let allowed = |c: char| {
        c.is_ascii_alphanumeric() || "#&+-()/.*'".contains(c) || (!c.is_ascii() && !c.is_control())
    };
    if name.chars().all(allowed) {
        return name;
    }
    let safe: String = name.chars().filter(|&c| allowed(c)).collect();
    eprintln!("Warning: the chord {name:?} holds characters no chord has; written as {safe:?}");
    safe
}

/// A comment ends at the end of its line, so text set in one must not break
/// the line: whatever followed the break would be read as LaTeX again.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Rewrites the flat of a note name for the `songs` package, which makes `&`
/// an active character printing a flat sign. A literal `Ab` would be set as the
/// letter b and would also hide the accidental from `\transpose`, so it has to
/// reach LaTeX as `A&`.
fn flat_note(note: &str) -> String {
    let mut chars = note.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some('b')) if letter.is_ascii_uppercase() => {
            format!("{}&{}", letter, chars.as_str())
        }
        _ => note.to_string(),
    }
}

/// Same for a chord quality, where a `b` only ever lowers a degree (`7b5`,
/// `mb5`, `b9`) — always directly in front of the degree number.
fn flat_degrees(modifier: &str) -> String {
    let mut out = String::with_capacity(modifier.len());
    let mut chars = modifier.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(next) if c == 'b' && next.is_ascii_digit() => out.push('&'),
            _ => out.push(c),
        }
    }
    out
}

/// The key is a chord name, so its accidentals want to be typeset as
/// accidentals. Outside the song environments `&` and `#` are not the package's
/// active characters, so the symbol macros have to be spelled out; the rest of
/// the value is escaped as usual.
fn escape_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut chars = key.chars().peekable();
    let mut prev = None;
    while let Some(c) = chars.next() {
        let is_flat = c == 'b'
            && (prev.is_some_and(|p: char| p.is_ascii_uppercase())
                || chars.peek().is_some_and(|n| n.is_ascii_digit()));
        match c {
            _ if is_flat => out.push_str("\\flt "),
            '#' => out.push_str("\\shrp "),
            _ => escape_char(c, &mut out),
        }
        prev = Some(c);
    }
    out
}

/// Escapes the characters LaTeX would otherwise read as markup. Lyrics are
/// full of them — `_` in "Ah!____", `&`, `%` and `#` in titles and URLs.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        escape_char(c, &mut out);
    }
    out
}

fn escape_char(c: char, out: &mut String) {
    match c {
        '\\' => out.push_str("\\textbackslash{}"),
        '~' => out.push_str("\\textasciitilde{}"),
        '^' => out.push_str("\\textasciicircum{}"),
        '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
            out.push('\\');
            out.push(c);
        }
        _ => out.push(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::PlacedChord;

    /// A lyric line with its chords, each given as (column, chord name).
    fn line(lyrics: &str, chords: &[(usize, &str)]) -> Line {
        Line {
            lyrics: lyrics.to_string(),
            chords: chords
                .iter()
                .map(|&(position, root)| PlacedChord {
                    chord: Chord {
                        root: root.to_string(),
                        modifier: None,
                        bass: None,
                    },
                    position,
                })
                .collect(),
        }
    }

    fn verse(lines: Vec<Line>) -> Part {
        Part {
            part_type: PartType::Verse,
            lines,
        }
    }

    /// Renders a whole part the way the printer does.
    fn render(part: &Part) -> Vec<String> {
        let marks = quote_marks(part);
        part.lines
            .iter()
            .zip(&marks)
            .map(|(line, marks)| render_line(line, marks))
            .collect()
    }

    #[test]
    fn a_quotation_on_one_line_is_a_group() {
        let part = verse(vec![line("He said \"no way\" and left", &[])]);
        assert_eq!(render(&part), ["He said \\enquote{no way} and left"]);
    }

    #[test]
    fn a_chord_in_front_of_a_quotation_is_set_inside_it() {
        let part = verse(vec![line(
            "   \"Sir, can you help me?\"",
            &[(0, "Em"), (17, "D")],
        )]);
        assert_eq!(
            render(&part),
            ["   \\enquote{\\[Em]Sir, can you \\[D]help me?}"]
        );
    }

    #[test]
    fn a_chord_behind_a_quotation_stays_outside_it() {
        let part = verse(vec![line("say \"no\"  and go", &[(8, "G")])]);
        assert_eq!(render(&part), ["say \\enquote{no} \\[G]  and go"]);
    }

    #[test]
    fn a_chord_on_the_closing_mark_is_sung_on_the_word_behind_it() {
        let part = verse(vec![line("say \"no\" and go", &[(7, "G")])]);
        assert_eq!(render(&part), ["say \\enquote{no} \\[G]and go"]);
    }

    #[test]
    fn a_quotation_over_several_lines_is_a_group_as_well() {
        let part = verse(vec![
            line("  \"Sir, can you help me?", &[(2, "Em")]),
            line("It's cold out here,", &[]),
            line("is there somewhere?\"", &[(19, "D")]),
        ]);
        assert_eq!(
            render(&part),
            [
                "  \\enquote{\\[Em]Sir, can you help me?",
                "It's cold out here,",
                "is there somewhere?}\\[D]",
            ]
        );
    }

    #[test]
    fn a_quotation_the_source_never_closes_stays_a_plain_mark() {
        let part = verse(vec![line("He said \"hello", &[])]);
        assert_eq!(render(&part), ["He said \\textquotedbl{}hello"]);
    }

    #[test]
    fn only_the_mark_left_over_stays_plain() {
        let part = verse(vec![
            line("say \"no\" and", &[]),
            line("(\"You! Stand still!)", &[(1, "Dm")]),
        ]);
        let rendered = render(&part);
        assert_eq!(rendered[0], "say \\enquote{no} and");
        // The chord moves past the mark onto the word it is sung on.
        assert_eq!(rendered[1], "(\\textquotedbl{}\\[Dm]You! Stand still!)");
    }

    #[test]
    fn every_part_keeps_its_braces_balanced() {
        for lyrics in ["\"a", "\"a\" \"b", "a\" b\" c\"", "\"\"\""] {
            let part = verse(vec![line(lyrics, &[]), line("x\"", &[])]);
            let text = render(&part).join("\n");
            assert_eq!(
                text.matches('{').count(),
                text.matches('}').count(),
                "{text}"
            );
        }
    }

    #[test]
    fn a_stray_mark_does_not_reach_into_the_next_part() {
        let first = verse(vec![line("\"Sir,", &[])]);
        let second = verse(vec![line("he walks on\"", &[])]);
        assert_eq!(render(&first), ["\\textquotedbl{}Sir,"]);
        assert_eq!(render(&second), ["he walks on\\textquotedbl{}"]);
    }

    #[test]
    fn chords_keep_their_place_where_no_quotation_is_in_the_way() {
        let part = verse(vec![line(
            "She calls out to the man",
            &[(0, "Em"), (10, "D")],
        )]);
        assert_eq!(render(&part), ["\\[Em]She calls \\[D]out to the man"]);
    }

    /// A chord landing on the space behind a word keeps a space of its own,
    /// so that the two do not run together.
    #[test]
    fn a_chord_on_a_space_is_kept_off_the_word_in_front_of_it() {
        let part = verse(vec![line("She calls out to the man", &[(9, "D")])]);
        assert_eq!(render(&part), ["She calls \\[D] out to the man"]);
    }

    /// "N.C." is the absence of a chord rather than one to be named, and the
    /// package has a macro that sets it — written where a name would be, in
    /// the column the mark was played in.
    #[test]
    fn the_no_chord_mark_is_set_with_the_packages_macro() {
        let part = verse(vec![line("and away we go", &[(0, "N.C."), (9, "G")])]);
        assert_eq!(render(&part), ["\\[\\nochord]and away \\[G]we go"]);
    }

    #[test]
    fn flats_become_ampersands() {
        assert_eq!(flat_note("Ab"), "A&");
        assert_eq!(flat_note("Bbmaj7"), "B&maj7");
        assert_eq!(flat_degrees("mi7b5"), "mi7&5");
        assert_eq!(flat_degrees("7b9"), "7&9");
    }

    #[test]
    fn the_key_spells_its_accidentals_out() {
        assert_eq!(escape_key("Ab"), "A\\flt ");
        assert_eq!(escape_key("F#mi"), "F\\shrp mi");
        assert_eq!(escape_key("Bbmaj7"), "B\\flt maj7");
        assert_eq!(escape_key("H"), "H");
        assert_eq!(escape_key("bez akordů"), "bez akordů");
    }

    #[test]
    fn other_chords_are_left_alone() {
        assert_eq!(flat_note("B"), "B");
        assert_eq!(flat_note("H"), "H");
        assert_eq!(flat_note("F#"), "F#");
        assert_eq!(flat_note("Es"), "Es");
        assert_eq!(flat_degrees("sus4"), "sus4");
        assert_eq!(flat_degrees("mi"), "mi");
    }

    fn song(meter: Option<&str>, capo: Option<u8>) -> Song {
        Song {
            title: "T".to_string(),
            artist: "A".to_string(),
            source_website: "https://example.org".to_string(),
            key: None,
            tempo: None,
            meter: meter.map(str::to_string),
            language: None,
            tuning: None,
            capo,
            extra: Vec::new(),
            parts: Vec::new(),
        }
    }

    fn header(song: &Song) -> String {
        let mut out = Vec::new();
        LatexPrinter.print(song, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn a_chord_cannot_carry_latex() {
        let part = verse(vec![line(
            "and away",
            &[(0, "]\\directlua{os.execute(\"rm -rf ~\")}\\[")],
        )]);
        let rendered = render(&part).join("\n");
        assert!(!rendered.contains("\\directlua"), "{rendered}");
        assert!(!rendered.contains('{'), "{rendered}");
        assert_eq!(rendered.matches("\\[").count(), 1, "{rendered}");
        assert_eq!(rendered.matches(']').count(), 1, "{rendered}");
    }

    #[test]
    fn real_chord_names_pass_unchanged() {
        for name in [
            "C", "F#m7", "Bbmaj7", "C#m7b5", "Dsus4", "A7(b9)", "G/H", "E°", "CΔ7", "Hmi",
        ] {
            let chord = Chord {
                root: name.to_string(),
                modifier: None,
                bass: None,
            };
            assert_eq!(chord_name(&chord), flat_note(name), "{name}");
        }
    }

    #[test]
    fn comments_stay_on_one_line() {
        let mut song = song(None, None);
        song.extra = vec![("Year\nX".to_string(), "1980\n\\directlua{x}".to_string())];
        song.parts = vec![Part {
            part_type: PartType::Other("solo\n\\directlua{x}".to_string()),
            lines: Vec::new(),
        }];
        let out = header(&song);
        for line in out.lines().filter(|l| l.contains("directlua")) {
            assert!(line.trim_start().starts_with('%'), "{line:?} in\n{out}");
        }
    }

    #[test]
    fn meter_and_capo_are_song_keys() {
        let out = header(&song(Some("3/4"), Some(2)));
        assert!(out.contains("    met={3/4},\n"));
        assert!(out.contains("    cap={2},\n"));
        assert!(!out.contains("\\capo"));
    }

    #[test]
    fn a_missing_meter_or_capo_is_left_as_a_comment() {
        let out = header(&song(None, Some(0)));
        assert!(out.contains("    % met={},\n"));
        assert!(out.contains("    % cap={},\n"));
    }

    #[test]
    fn an_intro_of_chords_alone_is_set_as_an_instrumental() {
        let line = |lyrics: &str| Line {
            lyrics: lyrics.to_string(),
            chords: vec![PlacedChord {
                chord: Chord {
                    root: "C".to_string(),
                    modifier: None,
                    bass: None,
                },
                position: 0,
            }],
        };
        let mut song = song(None, None);
        song.parts = vec![
            Part {
                part_type: PartType::Intro,
                lines: vec![line("| x2")],
            },
            Part {
                part_type: PartType::Intro,
                lines: vec![line("La la la")],
            },
        ];

        let out = header(&song);
        assert_eq!(out.matches("\\begininstrumental").count(), 1);
        assert_eq!(out.matches("\\beginintro").count(), 1);
        assert!(out.find("\\begininstrumental") < out.find("\\beginintro"));
    }

    /// Prints a song of the given parts and returns what stands between
    /// the prelude and `\endsong`.
    fn body(parts: Vec<Part>) -> String {
        let mut song = song(None, None);
        song.parts = parts;
        let out = header(&song);
        let start = out.find("]\n\n").unwrap() + 3;
        let end = out.find("\\endsong").unwrap();
        out[start..end].to_string()
    }

    fn part(part_type: PartType, lines: Vec<Line>) -> Part {
        Part { part_type, lines }
    }

    #[test]
    fn chords_without_bar_lines_get_no_bars_guessed() {
        let intro = part(
            PartType::Intro,
            vec![line("", &[(0, "G"), (6, "Gsus2"), (20, "G")])],
        );
        assert_eq!(
            body(vec![intro]),
            "\\begininstrumental\n    \\[G]\\[Gsus2]\\[G]\n\\endinstrumental\n\n"
        );
    }

    #[test]
    fn the_sources_bar_lines_are_kept() {
        let solo = part(
            PartType::Other("solo".to_string()),
            vec![line(
                "|       |       |       |",
                &[(2, "Am"), (5, "Em"), (10, "F"), (18, "E")],
            )],
        );
        assert_eq!(
            body(vec![solo]),
            "\\begininstrumental\n    | \\[Am]\\[Em] | \\[F]\\[] | \\[E]\\[] |\n\\endinstrumental\n\n"
        );
    }

    #[test]
    fn a_repeat_count_becomes_rep() {
        let outro = part(
            PartType::Outro,
            vec![
                line("|     |      | x2", &[(2, "C"), (8, "G")]),
                line("", &[(0, "Am")]),
                line("(4x)", &[]),
            ],
        );
        assert_eq!(
            body(vec![outro]),
            "\\begininstrumental\n    | \\[C]\\[] | \\[G]\\[] | \\rep{2}\n    \\[Am] \\rep{4}\n\\endinstrumental\n\n"
        );
    }

    #[test]
    fn a_row_is_repeated_once_however_often_it_says_so() {
        let outro = part(
            PartType::Outro,
            vec![
                line("|     |      | x2", &[(2, "C"), (8, "G")]),
                line("3x", &[]),
            ],
        );
        assert_eq!(
            body(vec![outro]),
            "\\begininstrumental\n    | \\[C]\\[] | \\[G]\\[] | \\rep{2}\n\\endinstrumental\n\n"
        );
    }

    #[test]
    fn a_bar_repeated_or_left_empty_keeps_its_place() {
        let solo = part(
            PartType::Instrumental,
            vec![line("|    | %  |    |    |", &[(2, "Am"), (17, "F")])],
        );
        assert_eq!(
            body(vec![solo]),
            "\\begininstrumental\n    | \\[Am]\\[] | \\[\\%]\\[] | \\[]\\[] | \\[F]\\[] |\n\\endinstrumental\n\n"
        );
    }

    #[test]
    fn a_sung_intro_stays_an_intro() {
        let intro = part(PartType::Intro, vec![line("La la", &[(0, "C")])]);
        assert_eq!(
            body(vec![intro]),
            "\\beginintro\n    \\[C]La la\n\\endintro\n\n"
        );
    }

    #[test]
    fn tablature_is_never_an_instrumental() {
        let tab = part(PartType::Tab, vec![line("", &[(0, "C")])]);
        assert!(body(vec![tab]).starts_with("\\beginverse*\n    % [TAB]\n"));
    }

    #[test]
    fn chords_after_the_last_word_are_one_space_apart() {
        let part = verse(vec![line(
            "to me   ",
            &[(3, "E&"), (6, "E&sus4"), (14, "E&")],
        )]);
        assert_eq!(render(&part), ["to \\[E&]me \\[E&sus4] \\[E&]"]);
    }

    #[test]
    fn a_chord_right_behind_the_last_word_stays_on_it() {
        let part = verse(vec![line("vzít", &[(4, "G")])]);
        assert_eq!(render(&part), ["vzít\\[G]"]);
    }

    #[test]
    fn part_content_is_indented_and_blank_lines_are_not() {
        let verse = verse(vec![line("one", &[]), line("", &[]), line("two", &[])]);
        assert_eq!(
            body(vec![verse]),
            "\\beginverse\n    one\n\n    two\n\\endverse\n\n"
        );
    }
}
