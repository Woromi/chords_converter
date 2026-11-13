use super::SongParser;
use super::text::{english_note_name, parse_chord, shorten_minor};
use crate::models::{Chord, Line, Part, PartType, PlacedChord, Song};
use scraper::{ElementRef, Html, Selector, node::Node};

/// Parses the chord sheets of akordy.kytary.cz.
///
/// The sheet is written out as HTML: a `<div class="scs-section">` per part,
/// carrying the part it is in its `data-type`, a `<div>` per line, and every
/// chord as a `<span class="scs-chord">` standing right in front of the letter
/// it is played over. The chord is drawn above the line and takes no column of
/// its own, so the letters around it keep the columns they are printed in.
pub struct KytaryCzParser;

impl SongParser for KytaryCzParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let document = Html::parse_document(html_content);

        let title_selector = Selector::parse(".sheet-title").unwrap();
        let author_selector = Selector::parse(".sheet-author a, .sheet-author u").unwrap();
        let section_selector = Selector::parse(".scs-section").unwrap();

        let title = document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "Unknown Title".to_string());

        // A song of two performers names both, each in a link of its own.
        let mut artists: Vec<String> = Vec::new();
        for el in document.select(&author_selector) {
            let name = el.text().collect::<String>().trim().to_string();
            if !name.is_empty() && !artists.contains(&name) {
                artists.push(name);
            }
        }
        let artist = match artists.is_empty() {
            true => "Unknown Artist".to_string(),
            false => artists.join(", "),
        };

        let mut parts: Vec<Part> = document
            .select(&section_selector)
            .map(parse_section)
            .filter(|part| !part.lines.is_empty())
            .collect();

        // The site names every chord the Czech way, an English song's too
        // (`B` for B flat, `H` for B), while the songbook reads a song's
        // chords in the song's own language. Only Czech and Slovak share the
        // site's naming; any other song's chords are renamed to match it.
        let language = parse_language(&document);
        if language
            .as_deref()
            .is_some_and(|l| l != "czech" && l != "slovak")
        {
            for chord in parts
                .iter_mut()
                .flat_map(|part| part.lines.iter_mut())
                .flat_map(|line| line.chords.iter_mut())
                .map(|placed| &mut placed.chord)
                .filter(|chord| !chord.is_no_chord())
            {
                chord.root = english_note_name(&chord.root);
                chord.bass = chord.bass.as_deref().map(english_note_name);
            }
        }

        if parts.is_empty() {
            eprintln!("Warning: no song found on {}.", url);
        }

        Song {
            title,
            artist,
            source_website: url.to_string(),
            key: None,   // Not provided by the site.
            tempo: None, // Not provided by the site.
            meter: None, // Not provided by the site.
            language,
            tuning: None, // Not provided by the site.
            capo: parse_capo(&document),
            extra: Vec::new(),
            parts,
        }
    }
}

/// Reads the language the sheet is sung in. The site marks it on the sheet
/// itself (`<div class="smartchordsheet" lang="en">`), for screen readers and
/// translation tools, and it is the song's language rather than the page's:
/// an English song sits on the Czech site with `lang="en"`.
fn parse_language(document: &Html) -> Option<String> {
    let sheet = Selector::parse(".smartchordsheet[lang]").ok()?;
    let code = document.select(&sheet).next()?.value().attr("lang")?;
    super::language_name(code)
}

/// Reads the fret the capo is on. The site writes it into its own menu, as
/// "Kapodastr — pražec 2"; a capo on the nut is no capo at all, which the
/// site has said (`Some(0)`).
fn parse_capo(document: &Html) -> Option<u8> {
    let items = Selector::parse(".overflow-item, .toolbar-group .group-content").ok()?;
    let value = Selector::parse(".overflow-value, .toolbar-value").ok()?;

    for item in document.select(&items) {
        let label: String = item.text().collect::<String>().to_lowercase();
        if !(label.contains("kapodastr") || label.contains("capo")) {
            continue;
        }
        let written: String = match item.select(&value).next() {
            Some(el) => el.text().collect(),
            None => label,
        };
        let fret: String = written
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(fret) = fret.parse::<u8>() {
            return Some(fret);
        }
    }

    None
}

fn parse_section(section_el: ElementRef) -> Part {
    let part_type = match section_el
        .attr("data-type")
        .unwrap_or("other")
        .to_lowercase()
        .as_str()
    {
        "intro" => PartType::Intro,
        "verse" => PartType::Verse,
        "prechorus" | "pre-chorus" => PartType::PreChorus,
        "chorus" | "refrain" => PartType::Chorus,
        "bridge" => PartType::Bridge,
        "instrumental" | "interlude" | "solo" => PartType::Instrumental,
        "tab" => PartType::Tab,
        "outro" | "ending" | "coda" => PartType::Outro,
        other => PartType::Other(other.to_string()),
    };

    let lines = section_el
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|el| el.value().name() == "div")
        .map(parse_line)
        .filter(|line| !line.lyrics.is_empty() || !line.chords.is_empty())
        .collect();

    Part { part_type, lines }
}

fn parse_line(line_el: ElementRef) -> Line {
    let mut lyrics = String::new();
    let mut chords = Vec::new();
    let mut position = 0;

    for child in line_el.children() {
        match child.value() {
            Node::Text(text_node) => {
                let chunk = text_node.replace('\u{a0}', " ");
                lyrics.push_str(&chunk);
                position += chunk.chars().count();
            }
            Node::Element(element_node) => {
                if let Some(child_el) = ElementRef::wrap(child) {
                    if element_node.classes().any(|c| c == "scs-chord") {
                        chords.push(PlacedChord {
                            chord: parse_chord_span(child_el),
                            position,
                        });
                    } else {
                        let chunk = child_el.text().collect::<String>().replace('\u{a0}', " ");
                        lyrics.push_str(&chunk);
                        position += chunk.chars().count();
                    }
                }
            }
            _ => {}
        }
    }

    // A line of nothing but chords — an intro or an interlude — is written
    // with the chords one after another, each a hair's breadth apart. Where
    // no word is left for them to stand over, they are laid out in reading
    // order instead, rather than piled into the first few columns.
    if lyrics.trim().is_empty() && !chords.is_empty() {
        return Line {
            lyrics: String::new(),
            chords: laid_out(chords),
        };
    }

    Line {
        lyrics: lyrics.trim_end().to_string(),
        chords,
    }
}

/// Spaces chords out, each two columns behind the one before it.
fn laid_out(chords: Vec<PlacedChord>) -> Vec<PlacedChord> {
    const GAP: usize = 2;

    let mut column = 0;
    chords
        .into_iter()
        .map(|placed| {
            let width = chord_width(&placed.chord);
            let placed = PlacedChord {
                position: column,
                ..placed
            };
            column += width + GAP;
            placed
        })
        .collect()
}

fn chord_width(chord: &Chord) -> usize {
    chord.root.chars().count()
        + chord.modifier.as_deref().map_or(0, |m| m.chars().count())
        + chord.bass.as_deref().map_or(0, |b| b.chars().count() + 1)
}

/// Reads a chord out of its span. The site sets the root, what follows it and
/// the bass note each in a span of its own; a page that stops doing so still
/// has the chord written out in full.
fn parse_chord_span(chord_el: ElementRef) -> Chord {
    let chk_sel = Selector::parse(".scs-chk").unwrap();
    let chv_sel = Selector::parse(".scs-chv").unwrap();
    let chb_sel = Selector::parse(".scs-chb").unwrap();

    let piece = |selector: &Selector| -> Option<String> {
        chord_el
            .select(selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|piece| !piece.is_empty())
    };

    match piece(&chk_sel) {
        Some(root) => shorten_minor(Chord {
            root,
            modifier: piece(&chv_sel),
            bass: piece(&chb_sel),
        }),
        None => parse_chord(chord_el.text().collect::<String>().trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps sections in the page the site would serve them in.
    fn page(capo: &str, sections: &str) -> String {
        format!(
            "<html><body><h1 class=\"sheet-title\">A Song</h1>\
             <h2 class=\"sheet-author\"><a href=\"#\"><u>A Band</u></a></h2>\
             <div class=\"overflow-item\"><span class=\"overflow-label\">Kapodastr</span>\
             <span class=\"overflow-value\">{}</span></div>\
             <div class=\"smartchordsheet\">{}</div></body></html>",
            capo, sections
        )
    }

    /// The site's markup for one chord.
    fn chord(root: &str, modifier: &str) -> String {
        let modifier = match modifier.is_empty() {
            true => String::new(),
            false => format!("<span class=\"scs-chv\">{}</span>", modifier),
        };
        format!(
            "<span class=\"scs-chord\"><span class=\"scs-chord-label\">\
             <span class=\"scs-chk\">{}</span>{}</span></span>",
            root, modifier
        )
    }

    fn parse(capo: &str, sections: &str) -> Song {
        KytaryCzParser.parse(
            "https://akordy.kytary.cz/song/a-song",
            &page(capo, sections),
        )
    }

    fn names(line: &Line) -> Vec<(String, usize)> {
        line.chords
            .iter()
            .map(|placed| {
                let mut name = placed.chord.root.clone();
                if let Some(modifier) = &placed.chord.modifier {
                    name.push_str(modifier);
                }
                (name, placed.position)
            })
            .collect()
    }

    #[test]
    fn a_chord_stands_over_the_letter_it_is_written_in_front_of() {
        let song = parse(
            "pražec 0",
            &format!(
                "<div class=\"scs-section\" data-type=\"verse\"><div>{}zmizela jako laň {}u lesa</div></div>",
                chord("D", "m"),
                chord("G", "7")
            ),
        );

        let line = &song.parts[0].lines[0];
        assert_eq!(line.lyrics, "zmizela jako laň u lesa");
        assert_eq!(names(line), [("Dm".to_string(), 0), ("G7".to_string(), 17)]);
    }

    #[test]
    fn the_part_is_the_one_the_section_says_it_is() {
        let song = parse(
            "pražec 0",
            &format!(
                "<div class=\"scs-section\" data-type=\"chorus\"><div>{}O lese</div></div>\
                 <div class=\"scs-section\" data-type=\"interlude\"><div>{}</div></div>\
                 <div class=\"scs-section\" data-type=\"ending\"><div>{}pam</div></div>",
                chord("A", "m"),
                chord("C", ""),
                chord("E", "")
            ),
        );

        assert!(matches!(song.parts[0].part_type, PartType::Chorus));
        assert!(matches!(song.parts[1].part_type, PartType::Instrumental));
        assert!(matches!(song.parts[2].part_type, PartType::Outro));
    }

    #[test]
    fn a_line_of_nothing_but_chords_is_laid_out_in_reading_order() {
        let song = parse(
            "pražec 0",
            &format!(
                "<div class=\"scs-section\" data-type=\"interlude\"><div>Na na na <br></div>\
                 <div>{}<char>&nbsp;</char>{}<char>&nbsp;</char>{}</div></div>",
                chord("A", "m"),
                chord("D", "m"),
                chord("G", "7")
            ),
        );

        let line = &song.parts[0].lines[1];
        assert_eq!(line.lyrics, "");
        assert_eq!(
            names(line),
            [
                ("Am".to_string(), 0),
                ("Dm".to_string(), 4),
                ("G7".to_string(), 8)
            ]
        );
    }

    /// The site sets the root and what follows it in spans of their own, so
    /// this chord never passes through the text parser and has to be
    /// shortened where it is built.
    #[test]
    fn a_minor_written_out_in_its_own_span_is_shortened() {
        let song = parse(
            "",
            &format!(
                "<div class=\"scs-section\" data-type=\"verse\"><div>{}Šel jsem</div></div>",
                chord("A", "mi7")
            ),
        );

        assert_eq!(names(&song.parts[0].lines[0]), [("Am7".to_string(), 0)]);
    }

    #[test]
    fn the_fret_the_capo_is_on_is_read_off_the_menu() {
        assert_eq!(parse("pražec 2", "").capo, Some(2));
        assert_eq!(parse("pražec 0", "").capo, Some(0));
    }

    #[test]
    fn a_page_carrying_no_song_still_parses() {
        let song = KytaryCzParser.parse("https://akordy.kytary.cz/", "<html></html>");

        assert_eq!(song.title, "Unknown Title");
        assert!(song.parts.is_empty());
    }

    #[test]
    fn the_language_is_read_off_the_sheet() {
        let sheet = |lang: &str| {
            format!(
                "<html><body><h1 class=\"sheet-title\">A Song</h1>\
                 <div class=\"smartchordsheet notranslate\" lang=\"{lang}\"></div></body></html>"
            )
        };
        let language = |lang: &str| {
            KytaryCzParser
                .parse("https://akordy.kytary.cz/song/a-song", &sheet(lang))
                .language
        };
        assert_eq!(language("cs").as_deref(), Some("czech"));
        assert_eq!(language("en").as_deref(), Some("english"));
        assert_eq!(language("sk").as_deref(), Some("slovak"));
        // A language the songbook does not typeset is left to a person.
        assert_eq!(language("de"), None);
    }

    #[test]
    fn a_sheet_that_names_no_language_leaves_it_open() {
        let song = parse("", "");
        assert_eq!(song.language, None);
    }

    #[test]
    fn an_english_songs_czech_chord_names_are_made_english() {
        let page = format!(
            "<html><body><h1 class=\"sheet-title\">A Song</h1>\
             <div class=\"smartchordsheet\" lang=\"{{}}\">\
             <div class=\"scs-section\" data-type=\"verse\"><div>{}one {}two {}three</div></div>\
             </div></body></html>",
            chord("B", ""),
            chord("H", "m7"),
            chord("Fis", "")
        );
        let chords = |lang: &str| {
            let song = KytaryCzParser.parse(
                "https://akordy.kytary.cz/song/a-song",
                &page.replace("{}", lang),
            );
            names(&song.parts[0].lines[0])
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        assert_eq!(chords("en"), ["Bb", "Bm7", "F#"]);
        // A Czech song keeps the names the songbook reads the Czech way.
        assert_eq!(chords("cs"), ["B", "Hm7", "Fis"]);
    }
}
