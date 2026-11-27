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
        if let Some(meter) = &song.meter {
            writeln!(writer, "Meter:    {}", meter)?;
        }
        if let Some(capo) = &song.capo {
            writeln!(writer, "Capo:     {}", capo)?;
        }
        if let Some(lang) = &song.language {
            writeln!(writer, "Language: {}", lang)?;
        }
        for (label, value) in &song.extra {
            writeln!(writer, "{}: {}", label, value)?;
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

                    // An instrumental line has no lyrics to print below it;
                    // an empty line here would split the part in two when the
                    // output is read back in.
                    if line.lyrics.is_empty() {
                        continue;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::{SongParser, text::TextParser};

    /// The text printer is the inverse of the text parser (CLAUDE.md): a
    /// song written out and read back is the same song. So a song in the
    /// printer's own layout, read and written out again, comes back byte
    /// for byte: the header and every kind of part, chords on their
    /// columns, a slash chord, the no-chord mark, a chords-only line, bars
    /// with a repeat count, tablature, and a part of a kind of its own.
    #[test]
    fn a_song_written_out_and_read_back_is_the_same() {
        let text = "\
Example Band - Morning Train
============================
Source:   https://example.org/song
Key:      Em
Tempo:    120
Tuning:   Standard
Meter:    3/4
Capo:     2
Language: english
Year: 1973

[Intro]
Em      D

[Verse]
Em          D
When we all go to the river
and back again

[Pre-Chorus]
     C         G
Over the hills we go

[Chorus]
N.C.    Am7/G
Row the boat, row

[Instrumental]
  C    G
|      | x2

[Tab]
e|--7--9--|
B|--8-----|

[SPOKEN]
Hello there

";
        let song = TextParser.parse("test", text);
        let mut out = Vec::new();
        TextPrinter.print(&song, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), text);
    }
}
