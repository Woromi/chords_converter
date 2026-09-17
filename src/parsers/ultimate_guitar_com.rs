use super::SongParser;
use super::text::parse_body;
use crate::models::Song;
use scraper::{Html, Selector};
use serde_json::Value;

/// Parses the chord sheets of ultimate-guitar.com.
///
/// The page holds no song in its HTML — it ships a JSON blob in
/// `<div class="js-store" data-content="...">` that the site's own scripts
/// render, and the sheet lives in it under
/// `store.page.data.tab_view.wiki_tab.content`.
///
/// That content is the familiar "chords over lyrics" layout dressed in the
/// site's markup: `[ch]..[/ch]` around every chord, `[tab]..[/tab]` around a
/// chord/lyric pair and `[Verse 1]`-style section markers. All four tags are
/// rendered as nothing at all, so dropping them leaves plain text with every
/// chord still in its original column — which is exactly what the shared text
/// parser reads.
pub struct UltimateGuitarComParser;

impl SongParser for UltimateGuitarComParser {
    fn parse(&self, url: &str, html_content: &str) -> Song {
        let store = extract_store(html_content);
        let data = store
            .as_ref()
            .and_then(|store| store.pointer("/store/page/data"));

        // Reads a non-empty string out of the page's JSON.
        let field = |path: &str| -> Option<String> {
            data.and_then(|data| data.pointer(path))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };

        let content = field("/tab_view/wiki_tab/content").unwrap_or_default();
        if content.is_empty() {
            eprintln!(
                "Warning: no song data found on {}. The site most likely answered \
                 with a bot check instead of the page.",
                url
            );
        }

        // The remark tabbers open with is not part of the song, but it is where
        // a capo or an alternate tuning tends to be mentioned.
        let title = field("/tab/song_name").unwrap_or_else(|| "Unknown Title".to_string());
        let (preamble, body) = split_preamble(&content);
        let notes = Preamble::read(&strip_markup(preamble), &title);

        let mut extra = Vec::new();
        if let Some(note) = notes.note {
            extra.push(("Note".to_string(), note));
        }
        if let Some(difficulty) = field("/tab/difficulty") {
            extra.push(("Difficulty".to_string(), difficulty));
        }
        if let Some(author) = field("/tab/username") {
            extra.push(("Tabber".to_string(), author));
        }

        Song {
            title,
            artist: field("/tab/artist_name").unwrap_or_else(|| "Unknown Artist".to_string()),
            source_website: field("/tab/tab_url").unwrap_or_else(|| url.to_string()),
            key: field("/tab/tonality_name")
                .or_else(|| field("/tab/recording/tonality_name"))
                .or(notes.key),
            tempo: notes.tempo,
            language: None, // Not provided by the site.
            tuning: notes.tuning,
            capo: notes.capo,
            extra,
            parts: parse_body(&strip_markup(body)),
        }
    }
}

/// Pulls the JSON the page keeps in the `data-content` attribute of its
/// `js-store` div. The attribute is HTML-escaped; the HTML parser hands it
/// back unescaped.
fn extract_store(html_content: &str) -> Option<Value> {
    let document = Html::parse_document(html_content);
    let store_sel = Selector::parse("div.js-store").ok()?;
    let raw = document
        .select(&store_sel)
        .next()?
        .value()
        .attr("data-content")?;
    serde_json::from_str(raw).ok()
}

/// Drops the site's zero-width markers. They take up no column when the page
/// is rendered, so removing them keeps the chords above the right syllables.
fn strip_markup(content: &str) -> String {
    const TAGS: [&str; 4] = ["[tab]", "[/tab]", "[ch]", "[/ch]"];

    let mut stripped = content.replace("\r\n", "\n").replace('\r', "\n");
    for tag in TAGS {
        stripped = stripped.replace(tag, "");
    }
    stripped
}

/// Splits the tabber's opening remark off the song. The song starts at the
/// first line that carries a chord, opens a `[tab]` block or names a section;
/// a sheet with no markup at all is taken to be song from the first line.
fn split_preamble(content: &str) -> (&str, &str) {
    let mut offset = 0;

    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        let starts_the_song = trimmed.contains("[ch]")
            || trimmed.starts_with("[tab]")
            || (trimmed.starts_with('[') && trimmed.ends_with(']'));

        if starts_the_song {
            return (&content[..offset], &content[offset..]);
        }
        offset += line.len();
    }

    ("", content)
}

/// The free text above the song — usually a remark from whoever tabbed it,
/// sometimes a `Capo: 2nd fret` or `Tuning: Drop D` line.
#[derive(Default)]
struct Preamble {
    key: Option<String>,
    tempo: Option<String>,
    tuning: Option<String>,
    capo: Option<u8>,
    note: Option<String>,
}

impl Preamble {
    fn read(text: &str, title: &str) -> Self {
        let mut preamble = Preamble::default();
        let mut remark: Vec<&str> = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            // The title and the rule under it are a heading, not a remark.
            if line.is_empty()
                || line.eq_ignore_ascii_case(title)
                || is_rule(line)
            {
                continue;
            }

            if let Some((label, value)) = labelled(line) {
                match label.as_str() {
                    "capo" => preamble.capo = leading_number(value),
                    "tuning" => preamble.tuning = Some(value.to_string()),
                    "tempo" | "bpm" => preamble.tempo = Some(value.to_string()),
                    _ => preamble.key = Some(value.to_string()),
                }
                continue;
            }

            // "Capo 2nd fret" — the one label tabs regularly write without a colon.
            if let Some(capo) = line
                .to_lowercase()
                .strip_prefix("capo")
                .and_then(leading_number)
            {
                preamble.capo = Some(capo);
                continue;
            }

            remark.push(line);
        }

        preamble.note = (!remark.is_empty()).then(|| remark.join(" "));
        preamble
    }
}

/// A row of `===` or `---` underlining the title above it.
fn is_rule(line: &str) -> bool {
    line.len() >= 3 && line.chars().all(|c| matches!(c, '=' | '-' | '_' | '~'))
}

/// Reads a `Capo: 2nd fret` line as a `(label, value)` pair, ignoring labels
/// the song model has no room for.
fn labelled(line: &str) -> Option<(String, &str)> {
    const LABELS: [&str; 6] = ["capo", "tuning", "key", "tonality", "tempo", "bpm"];

    let (head, value) = line.split_once(':')?;
    let label = head.trim().to_lowercase();
    LABELS
        .contains(&label.as_str())
        .then(|| (label, value.trim()))
}

/// Pulls the fret out of "2nd fret", "on the 3rd fret" or plain "2".
fn leading_number(value: &str) -> Option<u8> {
    value
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Line, PartType};

    /// Wraps a sheet in the page the site would serve it in, JSON blob and all.
    fn page(content: &str) -> String {
        let store = serde_json::json!({
            "store": { "page": { "data": {
                "tab": {
                    "song_name": "A Song",
                    "artist_name": "A Band",
                    "tonality_name": "Em",
                    "difficulty": "novice",
                    "username": "somebody",
                },
                "tab_view": { "wiki_tab": { "content": content } },
            }}}
        })
        .to_string();

        format!(
            "<html><body><div class=\"js-store\" data-content=\"{}\"></div></body></html>",
            store.replace('&', "&amp;").replace('"', "&quot;")
        )
    }

    fn parse(content: &str) -> Song {
        UltimateGuitarComParser.parse(
            "https://tabs.ultimate-guitar.com/tab/a-band/a-song-chords-1",
            &page(content),
        )
    }

    /// The chords of a line, each with the column it was placed in.
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
    fn reads_the_song_out_of_the_page_json() {
        let song = parse("[Verse 1]\r\n[tab][ch]Em[/ch]\r\nthe wind knows my name[/tab]");

        assert_eq!(song.title, "A Song");
        assert_eq!(song.artist, "A Band");
        assert_eq!(song.key.as_deref(), Some("Em"));
        assert!(song.extra.contains(&("Difficulty".to_string(), "novice".to_string())));
    }

    #[test]
    fn stripping_the_markup_leaves_the_chords_in_their_columns() {
        let song = parse("[Verse 1]\r\n[tab][ch]Em[/ch]      [ch]D/F#[/ch]\r\nthe wind knows my name[/tab]");

        let line = &song.parts[0].lines[0];
        assert_eq!(line.lyrics, "the wind knows my name");
        assert_eq!(
            chords(line),
            [("Em".to_string(), 0), ("D/F#".to_string(), 8)]
        );
    }

    #[test]
    fn section_markers_name_the_parts() {
        let song = parse(
            "[Intro]\r\n[tab][ch]Em[/ch] [ch]D[/ch][/tab]\r\n\r\n\
             [Chorus]\r\n[tab][ch]C[/ch]\r\nand away we go[/tab]",
        );

        assert!(matches!(song.parts[0].part_type, PartType::Intro));
        assert!(matches!(song.parts[1].part_type, PartType::Chorus));
    }

    #[test]
    fn the_opening_remark_is_kept_out_of_the_song() {
        let song = parse(
            "Capo: 2nd fret\r\nTuning: Drop D\r\nPlay it gently.\r\n\r\n\
             [Chorus]\r\n[tab][ch]C[/ch]\r\nand away we go[/tab]",
        );

        assert_eq!(song.capo, Some(2));
        assert_eq!(song.tuning.as_deref(), Some("Drop D"));
        assert_eq!(
            song.extra.first(),
            Some(&("Note".to_string(), "Play it gently.".to_string()))
        );
        // The remark is not a verse of its own.
        assert_eq!(song.parts.len(), 1);
        assert_eq!(song.parts[0].lines[0].lyrics, "and away we go");
    }

    #[test]
    fn a_capo_written_without_a_colon_is_still_read() {
        let song = parse("Capo 3rd fret\r\n\r\n[Verse]\r\n[tab][ch]C[/ch]\r\nand away we go[/tab]");

        assert_eq!(song.capo, Some(3));
        assert!(song.extra.iter().all(|(label, _)| label != "Note"));
    }

    #[test]
    fn a_page_carrying_no_song_still_parses() {
        let song = UltimateGuitarComParser.parse("https://tabs.ultimate-guitar.com/", "<html></html>");

        assert_eq!(song.title, "Unknown Title");
        assert!(song.parts.is_empty());
    }
}
