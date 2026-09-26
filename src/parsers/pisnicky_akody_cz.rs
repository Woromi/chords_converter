use super::SongParser;
use super::text::{parse_body, shorten_minor_name};
use crate::models::Song;
use scraper::{Html, Selector};

/// Parses the chord sheets of pisnicky-akordy.cz.
///
/// The song sits in a `<pre>` of its own, written the way a monospaced sheet
/// is: the chord lines are wrapped in `<el class="aline">` and every chord in
/// a `<span class="akord">`, but neither takes a column of its own. Reading
/// the text out of the block therefore gives the plain "chords over lyrics"
/// layout back, every chord in the column it is played in — which is what the
/// shared text parser reads.
pub struct PisnickyAkordyCzParser;

impl SongParser for PisnickyAkordyCzParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let document = Html::parse_document(html_content);

        let text_of = |selector: &str| -> Option<String> {
            let selector = Selector::parse(selector).ok()?;
            document
                .select(&selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .filter(|value| !value.is_empty())
        };

        // The sheet keeps every column it was written in — its first line is
        // a chord line, whose indentation says which syllable the chord sits
        // over — so only the blank lines around it are trimmed away.
        let sheet = Selector::parse("#songtext pre")
            .ok()
            .and_then(|selector| document.select(&selector).next())
            .map(|el| el.text().collect::<String>())
            .map(|sheet| sheet.trim_matches('\n').replace('\u{a0}', " "))
            .unwrap_or_else(|| {
                eprintln!("Warning: no song found on {}.", url);
                String::new()
            });

        // The key the sheet is written in; the page keeps it for its own
        // transposing buttons, which is why it reads it back from a field.
        let key = Selector::parse("input#tonina")
            .ok()
            .and_then(|selector| document.select(&selector).next())
            .and_then(|el| el.value().attr("value"))
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(shorten_minor_name);

        Song {
            title: text_of("#songheader h1").unwrap_or_else(|| "Unknown Title".to_string()),
            artist: text_of("#songheader h2").unwrap_or_else(|| "Unknown Artist".to_string()),
            source_website: url.to_string(),
            key,
            tempo: None,    // Not provided by the site.
            meter: None,    // Not provided by the site.
            language: None, // Not provided by the site, probably Czech.
            tuning: None,   // Not provided by the site.
            capo: None,     // Not provided by the site.
            extra: Vec::new(),
            parts: parse_body(&sheet),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Line, PartType};

    /// Wraps a sheet in the page the site would serve it in. A chord line is
    /// written as the site writes it, `<el>` and all.
    fn page(sheet: &str) -> String {
        let sheet = sheet
            .lines()
            .map(|line| match line.strip_prefix('~') {
                Some(chords) => {
                    let chords = chords.replace("<", "").replace(">", "");
                    let mut marked = String::new();
                    let mut rest = chords.as_str();
                    while let Some(start) = rest.find(|c: char| !c.is_whitespace()) {
                        let end = rest[start..]
                            .find(char::is_whitespace)
                            .map_or(rest.len(), |i| start + i);
                        marked.push_str(&rest[..start]);
                        marked
                            .push_str(&format!("<span class='akord'>{}</span>", &rest[start..end]));
                        rest = &rest[end..];
                    }
                    marked.push_str(rest);
                    format!(
                        "<el class='aline' style='display:inline;'>{}\n</el>",
                        marked
                    )
                }
                None => format!("{}\n", line),
            })
            .collect::<String>();

        format!(
            "<html><body><div id=\"songheader\"><h1><a href=\"#\">A Song</a></h1>\
             <h2><a href=\"#\">A Band</a></h2></div>\
             <input type=\"hidden\" id=\"tonina\" value=\"A\" />\
             <div id=\"songtext\"><pre>{}</pre></div></body></html>",
            sheet
        )
    }

    fn parse(sheet: &str) -> Song {
        PisnickyAkordyCzParser.parse("https://pisnicky-akordy.cz/a-band/a-song", &page(sheet))
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
    fn reads_the_song_and_the_key_off_the_page() {
        let song = parse("~     Ami\n1. Spatřil jsem kometu, oblohou letěla,");

        assert_eq!(song.title, "A Song");
        assert_eq!(song.artist, "A Band");
        assert_eq!(song.key.as_deref(), Some("A"));
        assert!(matches!(song.parts[0].part_type, PartType::Verse));
    }

    /// The page names the key the same way it names a chord.
    #[test]
    fn a_minor_key_is_named_the_short_way() {
        let page = page("~ Ami\n1. Spatřil jsem kometu").replace("value=\"A\"", "value=\"Ami\"");
        let song = PisnickyAkordyCzParser.parse("https://pisnicky-akordy.cz/a-band/a-song", &page);

        assert_eq!(song.key.as_deref(), Some("Am"));
    }

    #[test]
    fn a_chord_keeps_the_column_it_is_played_in() {
        let song = parse(
            "~     Ami            Dmi\n\
             1. Spatřil jsem kometu, oblohou letěla,\n\
             ~     C\n   \
             chtěl jsem jí zazpívat, ona mi zmizela,",
        );

        let lines = &song.parts[0].lines;
        assert_eq!(lines[0].lyrics, "Spatřil jsem kometu, oblohou letěla,");
        // "1. " gave its columns up, but the sheet's own margin went with it.
        assert_eq!(
            chords(&lines[0]),
            [("Am".to_string(), 2), ("Dm".to_string(), 17)]
        );
        assert_eq!(lines[1].lyrics, "chtěl jsem jí zazpívat, ona mi zmizela,");
        assert_eq!(chords(&lines[1]), [("C".to_string(), 2)]);
    }

    #[test]
    fn czech_chord_names_are_read_apart() {
        let song = parse("~ Cis  Hmi7  D/Fis\n lyrics");
        let names: Vec<_> = song.parts[0].lines[0]
            .chords
            .iter()
            .map(|placed| {
                (
                    placed.chord.root.clone(),
                    placed.chord.modifier.clone(),
                    placed.chord.bass.clone(),
                )
            })
            .collect();

        assert_eq!(
            names,
            [
                ("Cis".to_string(), None, None),
                ("H".to_string(), Some("m7".to_string()), None),
                ("D".to_string(), None, Some("Fis".to_string())),
            ]
        );
    }

    #[test]
    fn a_blank_line_starts_a_new_stanza() {
        let song = parse(
            "~   Ami\n1. Spatřil jsem kometu\n\n\
             2. Penízky ukryl jsem do hlíny\n\n\
             ~   Ami             Dmi\nR: O vodě, o trávě, o lese,",
        );

        assert_eq!(song.parts.len(), 3);
        assert!(matches!(song.parts[1].part_type, PartType::Verse));
        assert!(matches!(song.parts[2].part_type, PartType::Chorus));
        assert_eq!(song.parts[2].lines[0].lyrics, "O vodě, o trávě, o lese,");
    }

    #[test]
    fn an_interlude_keeps_its_brackets_and_no_lyrics() {
        let song = parse("~   Ami\nR: O vodě\n~   ( Dmi E )\n\n2. Penízky");

        let interlude = &song.parts[0].lines[1];
        assert_eq!(
            chords(interlude),
            [("Dm".to_string(), 2), ("E".to_string(), 6)]
        );
        assert_eq!(interlude.lyrics.trim(), "(       )");
    }

    #[test]
    fn a_page_carrying_no_song_still_parses() {
        let song = PisnickyAkordyCzParser.parse("https://pisnicky-akordy.cz/", "<html></html>");

        assert_eq!(song.title, "Unknown Title");
        assert!(song.parts.is_empty());
    }
}
