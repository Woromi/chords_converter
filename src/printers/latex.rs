use super::SongPrinter;
use crate::models::{PartType, Song};
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
                    writeln!(writer, "\\begin{{instrumental}}")?;
                    "\\end{instrumental}"
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

            // Process lines and merge chords inline
            for line in &part.lines {
                let chars: Vec<char> = line.lyrics.chars().collect();
                let mut line_out = String::new();
                let mut current_pos = 0;

                // Track whether we are inside a quote block on this line
                let mut in_quote = false;

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

                    // Catch up lyrics cursor to where this chord belongs
                    while current_pos < placed.position {
                        if current_pos < chars.len() {
                            let c = chars[current_pos];
                            if c == '"' {
                                if !in_quote {
                                    line_out.push_str("\\enquote{");
                                    in_quote = true;
                                } else {
                                    line_out.push('}');
                                    in_quote = false;
                                }
                            } else {
                                escape_char(c, &mut line_out);
                            }
                        } else {
                            // Pad with spaces if chord extends past existing lyrics
                            line_out.push(' ');
                        }
                        current_pos += 1;
                    }

                    // --- SPACING RULE CHECK ---
                    // If the chord lands exactly on a space immediately following a non-space (word),
                    // inject an extra space before placing the chord.
                    if placed.position > 0
                        && placed.position < chars.len()
                        && !chars[placed.position - 1].is_whitespace()
                        && chars[placed.position].is_whitespace()
                    {
                        line_out.push(' ');
                    }

                    // Inject the inline LaTeX chord
                    line_out.push_str(&format!("\\[{}]", chord_str));
                }

                // Push any remaining lyrics characters after the last chord
                while current_pos < chars.len() {
                    let c = chars[current_pos];
                    if c == '"' {
                        if !in_quote {
                            line_out.push_str("\\enquote{");
                            in_quote = true;
                        } else {
                            line_out.push('}');
                            in_quote = false;
                        }
                    } else {
                        escape_char(c, &mut line_out);
                    }
                    current_pos += 1;
                }

                // Guardrail: If a source line has an unclosed quote, force-close it
                // so it doesn't break the LaTeX compilation environment.
                if in_quote {
                    line_out.push('}');
                }

                writeln!(writer, "{}", line_out.trim_end())?;
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
