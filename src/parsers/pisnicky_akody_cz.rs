use super::SongParser;
use crate::models::{Chord, Line, Part, PartType, PlacedChord, Song};
use scraper::{ElementRef, Html, Selector};

pub struct PisnickyAkordyCzParser;

impl SongParser for PisnickyAkordyCzParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let document = Html::parse_document(html_content);

        // 1. Extract Metadata
        let title_sel = Selector::parse("#songheader h1").unwrap();
        let artist_sel = Selector::parse("#songheader h2").unwrap();
        let key_sel = Selector::parse("input#tonina").unwrap();

        let title = document
            .select(&title_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Title".to_string());

        let artist = document
            .select(&artist_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let key = document
            .select(&key_sel)
            .next()
            .and_then(|el| el.value().attr("value"))
            .map(|s| s.trim().to_string());

        // 2. Extract Parts, Lines, and Chords via DOM walk
        let mut parts: Vec<Part> = Vec::new();
        let mut current_lines: Vec<Line> = Vec::new();
        let mut current_part_type = PartType::Verse; // Default starting assumption
        let mut pending_chords: Vec<PlacedChord> = Vec::new();

        let pre_sel = Selector::parse("#songtext pre").unwrap();

        if let Some(pre_el) = document.select(&pre_sel).next() {
            for child in pre_el.children() {
                // A. If it's an <el> tag, it contains our chords for the next lyrics line
                if let Some(el) = child.value().as_element() {
                    if el.name() == "el" {
                        pending_chords = parse_el_chords(ElementRef::wrap(child).unwrap());
                    }
                }
                // B. If it's a text node, it contains lyrics (possibly multiple lines separated by \n)
                else if let Some(text_node) = child.value().as_text() {
                    let lines: Vec<&str> = text_node.split('\n').collect();

                    for raw_line in lines {
                        let trimmed = raw_line.trim();

                        // Skip purely empty lines unless we have pending chords waiting to attach
                        if trimmed.is_empty() && pending_chords.is_empty() {
                            continue;
                        }

                        let mut clean_lyrics = trimmed;

                        // Check if this line introduces a new part (e.g., "R:", "1.")
                        if let Some(detected_type) = detect_part_type(clean_lyrics) {
                            // Flush the previous part if it has accumulated lines
                            if !current_lines.is_empty() {
                                parts.push(Part {
                                    part_type: current_part_type,
                                    lines: std::mem::take(&mut current_lines),
                                });
                            }
                            current_part_type = detected_type;

                            // Clean the prefix marker (e.g., "R: text" becomes "text")
                            if let Some(idx) = clean_lyrics.find(|c| c == ':' || c == '.') {
                                clean_lyrics = clean_lyrics[idx + 1..].trim();
                            }
                        }

                        if clean_lyrics.is_empty() && pending_chords.is_empty() {
                            continue;
                        }

                        // --- THE FIX: Calculate exactly how many characters were stripped from the left ---
                        // Measure pointer difference to find the byte offset of where clean_lyrics starts
                        let byte_offset =
                            clean_lyrics.as_ptr() as usize - raw_line.as_ptr() as usize;
                        // Convert that byte slice into an exact Unicode character count
                        let chars_stripped_from_left = raw_line[..byte_offset].chars().count();

                        // Grab pending chords and shift their positions leftwards
                        let mut chords_to_attach = std::mem::take(&mut pending_chords);
                        for pc in &mut chords_to_attach {
                            // saturating_sub prevents underflow just in case a chord was placed over a stripped marker
                            pc.position = pc.position.saturating_sub(chars_stripped_from_left);
                        }

                        current_lines.push(Line {
                            lyrics: clean_lyrics.to_string(),
                            chords: chords_to_attach,
                        });
                    }
                }
            }

            // Flush the final remaining part
            if !current_lines.is_empty() {
                parts.push(Part {
                    part_type: current_part_type,
                    lines: current_lines,
                });
            }
        }

        Song {
            title,
            artist,
            source_website: url.to_string(),
            key,
            tempo: None,
            language: None, // Not provided by the site, probably czech
            tuning: None,
            capo: None,
            extra: Vec::new(),
            parts,
        }
    }
}

/// Walks inside an `<el>` tag to extract chords and calculate exact character offsets.
fn parse_el_chords(el: ElementRef) -> Vec<PlacedChord> {
    let mut chords = Vec::new();
    let mut current_pos = 0;

    for child in el.children() {
        if let Some(element) = child.value().as_element() {
            if element.name() == "span" {
                let chord_text = ElementRef::wrap(child).unwrap().text().collect::<String>();
                let trimmed_chord = chord_text.trim();

                if !trimmed_chord.is_empty() {
                    chords.push(PlacedChord {
                        chord: parse_chord(trimmed_chord),
                        position: current_pos,
                    });
                    // Advance our visual position index by the length of the chord string
                    current_pos += chord_text.chars().count();
                }
            }
        } else if let Some(text) = child.value().as_text() {
            // Count whitespace characters to track positional offset
            let cleaned = text.replace('\n', "");
            current_pos += cleaned.chars().count();
        }
    }

    chords
}

/// Splits a raw string like "D/F#" or "C#mi7" into a structured `Chord`.
fn parse_chord(s: &str) -> Chord {
    let parts: Vec<&str> = s.split('/').collect();
    let main_part = parts[0].trim();
    let bass = parts.get(1).map(|b| b.trim().to_string());

    // Comprehensive list of standard and Czech root nomenclature ordered by descending length
    // so we always match multi-character roots ("Cis", "C#") before single-character roots ("C").
    let possible_roots = [
        "Cis", "Dis", "Fis", "Gis", "Ais", "Ces", "Des", "Fes", "Ges", "C#", "D#", "F#", "G#",
        "A#", "Cb", "Db", "Eb", "Fb", "Gb", "Ab", "Bb", "Es", "As", "C", "D", "E", "F", "G", "A",
        "H", "B",
    ];

    let mut root = main_part
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let mut modifier = None;

    for r in possible_roots {
        if main_part.starts_with(r) {
            root = r.to_string();
            let mod_str = &main_part[r.len()..];
            if !mod_str.is_empty() {
                modifier = Some(mod_str.to_string());
            }
            break;
        }
    }

    Chord {
        root,
        modifier,
        bass,
    }
}

/// Identifies standard Czech/English structural markers at the start of a lyrics line.
fn detect_part_type(line: &str) -> Option<PartType> {
    let lower = line.to_lowercase();

    if lower.starts_with("r:") || lower.starts_with("ref:") || lower.starts_with("chorus") {
        Some(PartType::Chorus)
    } else if lower.starts_with("1.")
        || lower.starts_with("2.")
        || lower.starts_with("3.")
        || lower.starts_with("4.")
        || lower.starts_with("5.")
        || lower.starts_with("verse")
    {
        Some(PartType::Verse)
    } else if lower.starts_with("intro") || lower.starts_with("předehra") {
        Some(PartType::Intro)
    } else if lower.starts_with("mezihra")
        || lower.starts_with("bridge")
        || lower.starts_with("solo")
    {
        Some(PartType::Bridge)
    } else if lower.starts_with("outro") || lower.starts_with("dohra") {
        Some(PartType::Outro)
    } else {
        None
    }
}
