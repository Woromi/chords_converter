use super::SongPrinter;
use crate::models::{Line, Part, PartType, Song};
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
        match &song.tuning {
            Some(tuning) => writeln!(writer, "    tun={{{}}},", escape(tuning))?,
            None => writeln!(writer, "    tun={{Standard}},")?,
        }
        match &song.key {
            Some(key) => writeln!(writer, "    key={{{}}},", escape_key(key))?,
            None => writeln!(writer, "    % key={{}},")?,
        }
        writeln!(writer, "    src={{{}}},", escape(&song.source_website))?;
        // Metadata the song format has no key for is kept as a comment.
        for (label, value) in &song.extra {
            writeln!(
                writer,
                "    % {}={{{}}},",
                label.to_lowercase(),
                escape(value)
            )?;
        }
        writeln!(writer, "]")?;
        match &song.capo {
            None | Some(0) => writeln!(writer, "% \\capo{{}}")?,
            Some(capo) => writeln!(writer, "\\capo{{{}}}", capo)?,
        }
        writeln!(writer)?;

        // 2. Iterate through parts
        for part in &song.parts {
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
                    writeln!(writer, "% [TAB]")?;
                    "\\endverse*"
                }
                PartType::Outro => {
                    writeln!(writer, "\\beginoutro")?;
                    "\\endoutro"
                }
                PartType::Other(name) => {
                    writeln!(writer, "\\beginverse*")?;
                    writeln!(writer, "% [{}]", name.to_uppercase())?;
                    "\\endverse*"
                }
            };

            // Merge the chords into the lyrics, line by line. How a quotation
            // mark is set depends on where its counterpart is, so the whole
            // part is resolved before the first line is written.
            let marks = quote_marks(part);
            for (line, marks) in part.lines.iter().zip(&marks) {
                writeln!(writer, "{}", render_line(line, marks).trim_end())?;
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

/// How a `"` of the lyrics reaches LaTeX. `\enquote{…}` is the one to reach
/// for, but it is a group, and a group cannot be left open across a line: the
/// `songs` package sets every line for itself. A quotation running over
/// several lines therefore gets csquotes' unmatched pair, which prints the
/// very same marks without opening anything.
#[derive(Clone, Copy, Debug, PartialEq)]
enum QuoteMark {
    Open,
    Close,
    OpenSpanning,
    CloseSpanning,
}

impl QuoteMark {
    fn latex(self) -> &'static str {
        match self {
            QuoteMark::Open => "\\enquote{",
            QuoteMark::Close => "}",
            QuoteMark::OpenSpanning => "\\openautoquote{}",
            QuoteMark::CloseSpanning => "\\closeautoquote{}",
        }
    }
}

/// Resolves every `"` of a part, line by line and in reading order. A
/// quotation runs from each odd mark to the one after it; whether that one is
/// still on the same line decides how both of them are set. Pairing stops at
/// the part, so a stray mark cannot carry over into the rest of the song.
fn quote_marks(part: &Part) -> Vec<Vec<QuoteMark>> {
    // Every mark of the part as (line, its slot within that line).
    let mut places: Vec<(usize, usize)> = Vec::new();
    let mut marks: Vec<Vec<QuoteMark>> = Vec::with_capacity(part.lines.len());
    for (line_index, line) in part.lines.iter().enumerate() {
        let count = line.lyrics.matches('"').count();
        places.extend((0..count).map(|slot| (line_index, slot)));
        // Every slot is settled by the pass below; the value is a placeholder.
        marks.push(vec![QuoteMark::Open; count]);
    }

    for pair in places.chunks(2) {
        let (open_line, open_slot) = pair[0];
        let open = match pair.get(1) {
            Some(&(close_line, close_slot)) if close_line == open_line => {
                marks[close_line][close_slot] = QuoteMark::Close;
                QuoteMark::Open
            }
            Some(&(close_line, close_slot)) => {
                marks[close_line][close_slot] = QuoteMark::CloseSpanning;
                QuoteMark::OpenSpanning
            }
            // A quotation the source never closes. The unmatched mark prints
            // on its own, where `\enquote{` would have left a group open and
            // taken the rest of the song into it.
            None => QuoteMark::OpenSpanning,
        };
        marks[open_line][open_slot] = open;
    }

    marks
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
    let mut out = String::with_capacity(line.lyrics.len());
    let mut current_pos = 0;
    let mut next_mark = 0;

    // Ensure chords are processed left-to-right
    let mut sorted_chords = line.chords.clone();
    sorted_chords.sort_by_key(|c| c.position);

    for placed in &sorted_chords {
        // Build full chord string (Root + Modifier + Bass)
        let mut chord_str = flat_note(&placed.chord.root);
        if let Some(modifier) = &placed.chord.modifier {
            chord_str.push_str(&flat_degrees(modifier));
        }
        if let Some(bass) = &placed.chord.bass {
            chord_str.push('/');
            chord_str.push_str(&flat_note(bass));
        }

        // Catch the lyrics cursor up to the word this chord is sung on.
        let target = chord_target(&chars, placed.position);
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
    while current_pos < chars.len() {
        push_lyric(chars[current_pos], marks, &mut next_mark, &mut out);
        current_pos += 1;
    }

    out
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
            _ if is_flat => out.push_str("\\flt{}"),
            '#' => out.push_str("\\shrp{}"),
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

    use crate::models::{Chord, PlacedChord};

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
    fn a_quotation_over_several_lines_cannot_be_a_group() {
        let part = verse(vec![
            line("  \"Sir, can you help me?", &[(2, "Em")]),
            line("It's cold out here,", &[]),
            line("is there somewhere?\"", &[(19, "D")]),
        ]);
        assert_eq!(
            render(&part),
            [
                "  \\openautoquote{}\\[Em]Sir, can you help me?",
                "It's cold out here,",
                "is there somewhere?\\closeautoquote{}\\[D]",
            ]
        );
    }

    #[test]
    fn a_quotation_the_source_never_closes_leaves_no_group_open() {
        let part = verse(vec![line("He said \"hello", &[])]);
        assert_eq!(render(&part), ["He said \\openautoquote{}hello"]);
    }

    #[test]
    fn a_stray_mark_does_not_reach_into_the_next_part() {
        let first = verse(vec![line("\"Sir,", &[])]);
        let second = verse(vec![line("he walks on\"", &[])]);
        assert_eq!(render(&first), ["\\openautoquote{}Sir,"]);
        assert_eq!(render(&second), ["he walks on\\openautoquote{}"]);
    }

    #[test]
    fn chords_keep_their_place_where_no_quotation_is_in_the_way() {
        let part = verse(vec![line("She calls out to the man", &[(0, "Em"), (10, "D")])]);
        assert_eq!(render(&part), ["\\[Em]She calls \\[D]out to the man"]);
    }

    /// A chord landing on the space behind a word keeps a space of its own,
    /// so that the two do not run together.
    #[test]
    fn a_chord_on_a_space_is_kept_off_the_word_in_front_of_it() {
        let part = verse(vec![line("She calls out to the man", &[(9, "D")])]);
        assert_eq!(render(&part), ["She calls \\[D] out to the man"]);
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
        assert_eq!(escape_key("Ab"), "A\\flt{}");
        assert_eq!(escape_key("F#mi"), "F\\shrp{}mi");
        assert_eq!(escape_key("Bbmaj7"), "B\\flt{}maj7");
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
}
