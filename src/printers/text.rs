use super::SongPrinter;
use crate::models::{PartType, Song};
use std::io::Write;

pub struct TextPrinter;

impl SongPrinter for TextPrinter {
    fn print(&self, song: &Song, writer: &mut dyn Write) -> std::io::Result<()> {
        // 1. Print Main Header
        let header = format!("{} - {}", song.artist, song.title);
        writeln!(writer, "{}", header)?;
        writeln!(writer, "{}", "=".repeat(header.chars().count()))?;

        // 2. Print Metadata Block
        writeln!(writer, "Source:   {}", song.source_website)?;

        if let Some(key) = &song.key {
            writeln!(writer, "Key:      {}", key)?;
        }
        if let Some(tempo) = &song.tempo {
            writeln!(writer, "Tempo:    {}", tempo)?;
        }
        if let Some(tuning) = &song.tuning {
            writeln!(writer, "Tuning:   {}", tuning)?;
        }
        if let Some(capo) = &song.capo {
            writeln!(writer, "Capo:     {}", capo)?;
        }
        if let Some(lang) = &song.language {
            writeln!(writer, "Language: {}", lang)?;
        }
        writeln!(writer)?; // Empty line separating metadata from the lyrics

        // 3. Iterate through parts
        for part in &song.parts {
            let part_label = match &part.part_type {
                PartType::Intro => "[Intro]".to_string(),
                PartType::Verse => "[Verse]".to_string(),
                PartType::PreChorus => "[Pre-Chorus]".to_string(),
                PartType::Chorus => "[Chorus]".to_string(),
                PartType::Bridge => "[Bridge]".to_string(),
                PartType::Instrumental => "[Instrumental]".to_string(),
                PartType::Tab => "[Tab]".to_string(),
                PartType::Outro => "[Outro]".to_string(),
                PartType::Other(name) => format!("[{}]", name.to_uppercase()),
            };
            writeln!(writer, "{}", part_label)?;

            for line in &part.lines {
                if !line.chords.is_empty() {
                    let mut chord_line = String::new();
                    let mut current_char_pos = 0;

                    for placed in &line.chords {
                        let mut chord_str = placed.chord.root.clone();
                        if let Some(modifier) = &placed.chord.modifier {
                            chord_str.push_str(modifier);
                        }
                        if let Some(bass) = &placed.chord.bass {
                            chord_str.push('/');
                            chord_str.push_str(bass);
                        }

                        if placed.position > current_char_pos {
                            let spaces = placed.position - current_char_pos;
                            chord_line.push_str(&" ".repeat(spaces));
                            current_char_pos = placed.position;
                        } else if !chord_line.is_empty() {
                            chord_line.push(' ');
                            current_char_pos += 1;
                        }

                        chord_line.push_str(&chord_str);
                        current_char_pos += chord_str.chars().count();
                    }
                    writeln!(writer, "{}", chord_line)?;
                }

                writeln!(writer, "{}", line.lyrics)?;
            }
            writeln!(writer)?;
        }

        Ok(())
    }

    fn extension(&self) -> &str {
        "txt"
    }
}
