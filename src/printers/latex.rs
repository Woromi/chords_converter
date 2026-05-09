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
            None => writeln!(writer, "    % lang={{}},")?,
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
                            line_out.push(chars[current_pos]);
                        } else {
                            // Pad with spaces if chord extends past existing lyrics
                            line_out.push(' ');
                        }
                        current_pos += 1;
                    }

                    // Inject the inline LaTeX chord
                    line_out.push_str(&format!("\\[{}]", chord_str));
                }

                // Push any remaining lyrics characters after the last chord
                while current_pos < chars.len() {
                    line_out.push(chars[current_pos]);
                    current_pos += 1;
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
