use super::SongParser;
use crate::models::{Chord, Line, Part, PartType, PlacedChord, Song};
use scraper::{ElementRef, Html, Selector, node::Node};

pub struct KytaryCzParser;

impl SongParser for KytaryCzParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let document = Html::parse_document(html_content);

        // Core Selectors
        let title_selector = Selector::parse(".sheet-title").unwrap();
        let author_selector = Selector::parse(".sheet-author u").unwrap();
        let section_selector = Selector::parse(".scs-section").unwrap();

        // Metadata Selectors
        let capo_group_sel = Selector::parse(".toolbar-group .group-content").unwrap();

        // Extract Title & Artist
        let title = document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Title".to_string());

        let artist = document
            .select(&author_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Artist".to_string());

        // Extract Capo (Valid)
        let mut capo = None;
        for group in document.select(&capo_group_sel) {
            let text: String = group.text().collect();
            if text.contains("CAPO") {
                let val_sel = Selector::parse(".toolbar-value").unwrap();
                if let Some(val_el) = group.select(&val_sel).next() {
                    capo = val_el.text().collect::<String>().trim().parse::<u8>().ok();
                }
            }
        }

        // Extract Parts
        let mut parts = Vec::new();
        for section_el in document.select(&section_selector) {
            let part_type_str = section_el.attr("data-type").unwrap_or("other");
            let part_type = match part_type_str.to_lowercase().as_str() {
                "intro" => PartType::Intro,
                "verse" => PartType::Verse,
                "prechorus" | "pre-chorus" => PartType::PreChorus,
                "chorus" => PartType::Chorus,
                "bridge" => PartType::Bridge,
                "instrumental" | "interlude" | "solo" => PartType::Instrumental,
                "tab" => PartType::Tab,
                "outro" | "ending" => PartType::Outro,
                other => PartType::Other(other.to_string()),
            };

            let mut lines = Vec::new();
            for line_node in section_el.children() {
                if let Some(line_el) = ElementRef::wrap(line_node) {
                    if line_el.value().name() == "div" {
                        let parsed_line = parse_line(line_el);
                        if !parsed_line.lyrics.is_empty() || !parsed_line.chords.is_empty() {
                            lines.push(parsed_line);
                        }
                    }
                }
            }
            parts.push(Part { part_type, lines });
        }

        Song {
            title,
            artist,
            source_website: url.to_string(),
            key: None,      // Grounded in reality: not provided by the site
            tempo: None,    // Not provided by the site
            language: None, // UI language != Lyrics language
            tuning: None,   // Not provided by the site
            capo,           // Successfully extracted!
            parts,
        }
    }
}

// Private helper functions
fn parse_line(line_el: ElementRef) -> Line {
    let mut lyrics = String::new();
    let mut chords = Vec::new();
    let mut current_char_position = 0;

    for child in line_el.children() {
        match child.value() {
            Node::Text(text_node) => {
                let chunk = text_node.replace('\u{a0}', " ");
                lyrics.push_str(&chunk);
                current_char_position += chunk.chars().count();
            }
            Node::Element(element_node) => {
                if let Some(child_el) = ElementRef::wrap(child) {
                    if element_node.classes().any(|c| c == "scs-chord") {
                        let structured_chord = parse_chord(child_el);
                        chords.push(PlacedChord {
                            chord: structured_chord,
                            position: current_char_position,
                        });
                    } else {
                        let chunk = child_el.text().collect::<String>().replace('\u{a0}', " ");
                        lyrics.push_str(&chunk);
                        current_char_position += chunk.chars().count();
                    }
                }
            }
            _ => {}
        }
    }

    Line {
        lyrics: lyrics.trim_end().to_string(),
        chords,
    }
}

fn parse_chord(chord_el: ElementRef) -> Chord {
    let chk_sel = Selector::parse(".scs-chk").unwrap();
    let chv_sel = Selector::parse(".scs-chv").unwrap();
    let chb_sel = Selector::parse(".scs-chb").unwrap();

    let root = chord_el
        .select(&chk_sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_else(|| "??".to_string());

    let modifier = chord_el
        .select(&chv_sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    let bass = chord_el
        .select(&chb_sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    Chord {
        root,
        modifier,
        bass,
    }
}
