use super::SongParser;
use crate::models::{Chord, Line, Part, PartType, PlacedChord, Song};
use scraper::{Html, Selector};

pub struct BlivnikCzParser;

impl SongParser for BlivnikCzParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let document = Html::parse_document(html_content);

        // Metadata selectors
        let name_sel = Selector::parse("div#origName").unwrap();
        let auth_sel = Selector::parse("div#origAuth").unwrap();
        let text_sel = Selector::parse("div#origText").unwrap();

        let title = document
            .select(&name_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Title".to_string());

        let artist = document
            .select(&auth_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let raw_text = document
            .select(&text_sel)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_default();

        let mut parts = Vec::new();
        let mut current_lines = Vec::new();
        let mut current_part_type = PartType::Verse;
        let mut pending_chords: Vec<PlacedChord> = Vec::new();

        // Normalize non-breaking spaces (\u{a0}) to standard spaces, then split lines
        let lines: Vec<String> = raw_text
            .split('\n')
            .map(|l| l.replace('\u{a0}', " "))
            .collect();

        for raw_line in &lines {
            let trimmed = raw_line.trim();

            // Skip empty lines and the raw text metadata header
            if trimmed.is_empty() || trimmed == "[TEXT:]" {
                continue;
            }

            // A. If the line consists entirely of bracketed chords and spaces, parse chords
            if is_chord_line(raw_line) {
                let mut in_bracket = false;
                let mut chord_start = 0;
                let mut current_chord = String::new();

                for (idx, c) in raw_line.char_indices() {
                    if c == '[' {
                        in_bracket = true;
                        // Measure exact character position up to the opening bracket
                        chord_start = raw_line[..idx].chars().count();
                        current_chord.clear();
                    } else if c == ']' && in_bracket {
                        in_bracket = false;
                        let cleaned = current_chord.trim();
                        if !cleaned.is_empty() {
                            // Replace parse_chord with your crate's actual chord constructor
                            if let Some(chord) = parse_chord(cleaned) {
                                pending_chords.push(PlacedChord {
                                    chord,
                                    position: chord_start,
                                });
                            }
                        }
                    } else if in_bracket {
                        current_chord.push(c);
                    }
                }
                continue;
            }

            // B. Otherwise, process as a lyrics line
            let mut clean_lyrics = trimmed;

            // Check if this line introduces a new song part (e.g., "(1)", "(R)")
            if let Some((detected_type, marker_end_byte)) = detect_part_type(clean_lyrics) {
                // Flush accumulated lines into the previous part
                if !current_lines.is_empty() {
                    parts.push(Part {
                        part_type: current_part_type,
                        lines: std::mem::take(&mut current_lines),
                    });
                }
                current_part_type = detected_type;
                clean_lyrics = clean_lyrics[marker_end_byte..].trim();
            }

            if clean_lyrics.is_empty() && pending_chords.is_empty() {
                continue;
            }

            // --- THE OFFSET FIX ---
            // Calculate exactly how many characters were stripped (indentation spaces + markers)
            let byte_offset = clean_lyrics.as_ptr() as usize - raw_line.as_ptr() as usize;
            let chars_stripped_from_left = raw_line[..byte_offset].chars().count();

            // Grab pending chords from the line above and shift them leftwards
            let mut chords_to_attach = std::mem::take(&mut pending_chords);
            for pc in &mut chords_to_attach {
                pc.position = pc.position.saturating_sub(chars_stripped_from_left);
            }

            current_lines.push(Line {
                lyrics: clean_lyrics.to_string(),
                chords: chords_to_attach,
            });
        }

        // Flush the final part
        if !current_lines.is_empty() {
            parts.push(Part {
                part_type: current_part_type,
                lines: current_lines,
            });
        }

        Song {
            title,
            artist,
            parts,
            language: None,
            tempo: None,
            tuning: None,
            key: None,
            capo: None,
            extra: Vec::new(),
            source_website: url.to_string(),
        }
    }
}

/// Validates if a line only contains whitespace outside of `[...]` blocks.
fn is_chord_line(line: &str) -> bool {
    if !line.contains('[') || !line.contains(']') {
        return false;
    }
    let mut in_bracket = false;
    for c in line.chars() {
        if c == '[' {
            in_bracket = true;
        } else if c == ']' {
            in_bracket = false;
        } else if !in_bracket && !c.is_whitespace() {
            return false;
        }
    }
    true
}

/// Extracts part types from markers like `(1)`, `(R)`, `(I)`
fn detect_part_type(line: &str) -> Option<(PartType, usize)> {
    if line.starts_with('(') {
        if let Some(end_idx) = line.find(')') {
            let marker = line[1..end_idx].trim().to_uppercase();
            let part_type = match marker.as_str() {
                "I" | "INTRO" => PartType::Intro,
                "R" | "REF" | "CHORUS" => PartType::Chorus,
                "B" | "BRIDGE" => PartType::Bridge,
                "O" | "OUTRO" => PartType::Outro,
                m if m.chars().all(|c| c.is_ascii_digit()) => PartType::Verse,
                other => PartType::Other(other.to_string()),
            };
            // Return the detected type and the byte offset just after the ')'
            return Some((part_type, end_idx + 1));
        }
    }
    None
}

/// Stub for your existing chord parsing logic
fn parse_chord(s: &str) -> Option<Chord> {
    // Convert local minor format (e.g., "Ami", "Emi7") to standard ("Am", "Em7")
    let normalized = s.replace("mi", "m");

    // Wire up your actual chord string parser here using the normalized string
    Some(Chord {
        root: normalized,
        modifier: None,
        bass: None,
    })
}
