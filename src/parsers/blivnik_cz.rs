use super::SongParser;
use super::text::{parse_body, shorten_minor_name};
use crate::models::Song;
use scraper::{Html, Selector};

/// Parses the chord sheets of blivnik.cz.
///
/// The page ships the song as the site's own source text in a hidden
/// `<div id="origText">`, which its scripts then draw; everything the reader
/// sees is built from it. The format is:
///
/// ```text
/// [CAPO:2]
/// [Ami](X02210)
/// [INTRO:]
/// e|---3---|
/// [TEXT:]
///   [Ami]
/// (1)Jede šrot, jede šrot
/// ```
///
/// A header of capo, chord diagrams and tab blocks, then `[TEXT:]` and the
/// song itself. The site draws `[` and `]` as a space each, so replacing them
/// with spaces leaves the familiar "chords over lyrics" layout with every
/// chord in the column it is played in — which is what the shared text parser
/// reads.
pub struct BlivnikCzParser;

impl SongParser for BlivnikCzParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let document = Html::parse_document(html_content);

        let field = |id: &str| -> Option<String> {
            let selector = Selector::parse(&format!("div#{}", id)).ok()?;
            document
                .select(&selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|value| !value.is_empty())
        };

        let raw_text = field("origText").unwrap_or_default();
        if raw_text.is_empty() {
            eprintln!("Warning: no song found on {}.", url);
        }

        let normalised = raw_text.replace("\r\n", "\n").replace('\u{a0}', " ");
        let (head, body) = match normalised.split_once("[TEXT:]") {
            Some((head, body)) => (head, body),
            // A sheet with no marker at all is song from the first line.
            None => ("", normalised.as_str()),
        };
        let head = Head::read(head);

        let mut extra = Vec::new();
        if let Some(note) = head.note {
            extra.push(("Note".to_string(), note));
        }
        if !head.diagrams.is_empty() {
            extra.push(("Chord diagrams".to_string(), head.diagrams.join(", ")));
        }
        if let Some(date) = field("origDate") {
            extra.push(("Added".to_string(), date));
        }

        // Whatever the header held besides its metadata — an `[INTRO:]` and
        // its tablature, as a rule — is part of the song and belongs above it.
        let sheet = format!("{}\n{}", head.prelude.join("\n"), render(body));

        Song {
            title: field("origName").unwrap_or_else(|| "Unknown Title".to_string()),
            artist: field("origAuth").unwrap_or_else(|| "Unknown Artist".to_string()),
            source_website: url.to_string(),
            key: None,      // Not provided by the site.
            tempo: None,    // Not provided by the site.
            meter: None,    // Not provided by the site.
            language: None, // Not provided by the site, probably Czech.
            tuning: None,   // Not provided by the site.
            capo: head.capo,
            extra,
            parts: parse_body(&sheet),
        }
    }
}

/// What the site keeps above `[TEXT:]`.
#[derive(Default)]
struct Head {
    capo: Option<u8>,
    /// Chord diagrams, `[Ami](X02210)` in the source, kept as "Ami (X02210)".
    diagrams: Vec<String>,
    /// The `[INTRO:]` blocks and their tablature — song, not metadata.
    prelude: Vec<String>,
    /// A remark left by whoever wrote the sheet.
    note: Option<String>,
}

impl Head {
    fn read(text: &str) -> Self {
        let mut head = Head::default();
        let mut remark: Vec<&str> = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                // A blank line only matters where it separates two blocks of
                // the song; on its own it says nothing.
                if !head.prelude.is_empty() {
                    head.prelude.push(String::new());
                }
                continue;
            }

            if let Some(capo) = capo_of(trimmed) {
                head.capo = Some(capo);
            } else if let Some(diagram) = diagram_of(trimmed) {
                head.diagrams.push(diagram);
            } else if is_section_marker(trimmed) || looks_like_tablature(trimmed) {
                head.prelude.push(line.to_string());
            } else {
                remark.push(trimmed);
            }
        }

        head.note = (!remark.is_empty()).then(|| remark.join(" "));
        head
    }
}

/// Reads the fret out of a `[CAPO:2]` line, ignoring a capo on the nut.
fn capo_of(line: &str) -> Option<u8> {
    let inner = line.strip_prefix("[CAPO:")?.strip_suffix(']')?;
    inner.trim().parse().ok().filter(|fret| *fret > 0)
}

/// Reads a `[Ami](X02210)` chord diagram as "Ami (X02210)".
fn diagram_of(line: &str) -> Option<String> {
    let (chord, fingering) = line.strip_prefix('[')?.split_once("](")?;
    let fingering = fingering.strip_suffix(')')?;
    let plays = |c: char| c.is_ascii_digit() || c == 'X' || c == 'x' || c == ',';

    (!chord.is_empty()
        && !chord.contains(char::is_whitespace)
        && !fingering.is_empty()
        && fingering.chars().all(plays))
    .then(|| format!("{} ({})", shorten_minor_name(chord), fingering))
}

/// A `[INTRO:]`-style block heading.
fn is_section_marker(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(":]")
}

/// One string of a tablature staff, "e|--3--|". Close enough for the header,
/// where the only other thing it could be is a remark.
fn looks_like_tablature(line: &str) -> bool {
    matches!(line.split_once('|'), Some((string, body))
        if string.chars().count() <= 2 && body.matches('-').count() >= 4)
}

/// Draws the song the way the page does: `[` and `]` become a space each, so
/// every chord keeps the column it was written in and the line below it still
/// lines up. A bracket holding anything but a chord — a `[INTRO:]` heading —
/// is left for the text parser to read as the section marker it is.
fn render(body: &str) -> String {
    body.lines()
        .map(|line| {
            if is_section_marker(line.trim()) {
                return line.to_string();
            }

            let mut out = String::with_capacity(line.len());
            let mut rest = line;
            while let Some(open) = rest.find('[') {
                match rest[open..].find(']') {
                    Some(close) => {
                        let chord = &rest[open + 1..open + close];
                        out.push_str(&rest[..open]);
                        if chord.is_empty() || chord.contains(char::is_whitespace) {
                            out.push_str(&rest[open..open + close + 1]);
                        } else {
                            out.push(' ');
                            out.push_str(chord);
                            out.push(' ');
                        }
                        rest = &rest[open + close + 1..];
                    }
                    None => break,
                }
            }
            out.push_str(rest);
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Line, PartType};

    /// Wraps a sheet in the page the site would serve it in.
    fn page(text: &str) -> String {
        format!(
            "<html><body><div id=\"hiddenText\">\
             <div id=\"origText\">{}</div>\
             <div id=\"origName\">A Song</div>\
             <div id=\"origAuth\">A Band</div>\
             <div id=\"origDate\">11. 07. 2014</div>\
             </div></body></html>",
            text
        )
    }

    fn parse(text: &str) -> crate::models::Song {
        BlivnikCzParser.parse("https://www.blivnik.cz/song/1/", &page(text))
    }

    fn chords(line: &Line) -> Vec<(String, usize)> {
        line.chords
            .iter()
            .map(|placed| {
                let mut name = placed.chord.root.clone();
                if let Some(modifier) = &placed.chord.modifier {
                    name.push_str(modifier);
                }
                if let Some(bass) = &placed.chord.bass {
                    name.push('/');
                    name.push_str(bass);
                }
                (name, placed.position)
            })
            .collect()
    }

    #[test]
    fn reads_the_song_out_of_the_hidden_source() {
        let song = parse("[TEXT:]\n  [Ami]\n(1)Jede šrot, jede šrot");

        assert_eq!(song.title, "A Song");
        assert_eq!(song.artist, "A Band");
        assert!(matches!(song.parts[0].part_type, PartType::Verse));
        let line = &song.parts[0].lines[0];
        assert_eq!(line.lyrics, "Jede šrot, jede šrot");
        // "  [Ami]" draws as "   Ami ", above the "J" the marker gave up —
        // the column is the one the source wrote, the spelling ours.
        assert_eq!(chords(line), [("Am".to_string(), 0)]);
    }

    #[test]
    fn the_bass_note_of_a_chord_is_read_as_one() {
        let song = parse("[TEXT:]\n [C/G]      [F#/Hmi]\n Povím vám jednu baladu");

        assert_eq!(song.parts[0].lines[0].lyrics, "Povím vám jednu baladu");
        assert_eq!(
            chords(&song.parts[0].lines[0]),
            [("C/G".to_string(), 1), ("F#/Hm".to_string(), 12)]
        );
    }

    #[test]
    fn the_header_is_metadata_not_a_verse() {
        let song =
            parse("[CAPO:2]\n[Ami](X02210)\n[Dsus2](XX0230)\nZapsal Franta\n[TEXT:]\n [C]\n Ave");

        assert_eq!(song.capo, Some(2));
        assert_eq!(
            song.extra,
            [
                ("Note".to_string(), "Zapsal Franta".to_string()),
                (
                    "Chord diagrams".to_string(),
                    "Am (X02210), Dsus2 (XX0230)".to_string()
                ),
                ("Added".to_string(), "11. 07. 2014".to_string()),
            ]
        );
        assert_eq!(song.parts.len(), 1);
        assert_eq!(song.parts[0].lines[0].lyrics, "Ave");
    }

    #[test]
    fn a_capo_on_the_nut_is_no_capo() {
        assert_eq!(parse("[CAPO:0]\n[TEXT:]\n Ave").capo, None);
    }

    #[test]
    fn the_tablature_of_the_header_opens_the_song() {
        let song = parse(
            "[INTRO:]\ne|----------3---2-0---|\nh|--------------------|\n[TEXT:]\n [C]\n Ave",
        );

        assert!(matches!(song.parts[0].part_type, PartType::Tab));
        assert_eq!(song.parts[0].lines.len(), 2);
        assert!(song.parts[0].lines[0].lyrics.starts_with("e|"));
        assert_eq!(song.parts[1].lines[0].lyrics, "Ave");
    }

    #[test]
    fn a_bracketed_marker_names_the_part_it_opens() {
        let song = parse("[TEXT:]\n(R) O vodě, o trávě\n\n(I) [A] [E]\n\n(1) Spatřil jsem");

        assert!(matches!(song.parts[0].part_type, PartType::Chorus));
        assert_eq!(song.parts[0].lines[0].lyrics, "O vodě, o trávě");
        assert!(matches!(song.parts[1].part_type, PartType::Intro));
        assert_eq!(chords(&song.parts[1].lines[0]).len(), 2);
        assert!(matches!(song.parts[2].part_type, PartType::Verse));
    }

    #[test]
    fn a_page_carrying_no_song_still_parses() {
        let song = BlivnikCzParser.parse("https://www.blivnik.cz/", "<html></html>");

        assert_eq!(song.title, "Unknown Title");
        assert!(song.parts.is_empty());
    }
}
