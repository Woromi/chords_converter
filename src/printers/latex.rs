use super::SongPrinter;
use crate::models::{PartType, Song};
use std::io::Write;

pub struct LatexPrinter;

impl SongPrinter for LatexPrinter {
    fn print(&self, song: &Song, writer: &mut dyn Write) -> std::io::Result<()> {
        // 1. Begin Song & Metadata Block
        writeln!(writer, "\\beginsong{{{}}}[", song.title)?;
        writeln!(writer, "    by={{{}}},", song.artist)?;

        match &song.language {
            Some(lang) => writeln!(writer, "    lang={{{}}},", lang)?,
            None => writeln!(writer, "    % lang={{czech|english}},")?,
        }
        match &song.tempo {
            Some(tempo) => writeln!(writer, "    tem={{{}}},", tempo)?,
            None => writeln!(writer, "    % tem={{}},")?,
        }
        match &song.tuning {
            Some(tuning) => writeln!(writer, "    tun={{{}}},", tuning)?,
            None => writeln!(writer, "    tun={{Standard}},")?,
        }
        match &song.key {
            Some(key) => writeln!(writer, "    key={{{}}},", key)?,
            None => writeln!(writer, "    % key={{}},")?,
        }
        writeln!(writer, "    src={{{}}},", song.source_website)?;
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
                    let mut chord_str = placed.chord.root.clone();
                    if let Some(modifier) = &placed.chord.modifier {
                        chord_str.push_str(modifier);
                    }
                    if let Some(bass) = &placed.chord.bass {
                        chord_str.push('/');
                        chord_str.push_str(bass);
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
                                line_out.push(c);
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
                        line_out.push(c);
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
