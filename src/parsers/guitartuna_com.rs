use super::SongParser;
use super::text::{meter_of, parse_body, shorten_minor_name};
use crate::models::Song;
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::Value;

/// Parses the chord sheets of guitartuna.com.
///
/// The page lays the song out by the beat rather than by the column: every
/// `<div class="line">` is a row of beats, the beats a chord falls on carry it
/// in a `data-chord` attribute, and the lyrics of the line follow in a `<div>`
/// of their own. What ties the two together is the lyrics: for a screen reader
/// the site writes each chord out into them — `<span class="sr-only"> (B flat
/// major) </span>` — right where it is played. Those spans name the chords in
/// the order the beats do, so each one gives the column the next chord stands
/// in, and the line can be written back out as the "chords over lyrics" layout
/// the shared text parser reads.
///
/// Sections are headed by a `<div class="part_type">` ("Verse 1", "Bridge 2"),
/// which becomes a `[Verse 1]` marker. What the site prints above the sheet —
/// the key, the tempo, the tuning — it also hands search engines as JSON-LD,
/// which is where it is read from; the time signature rides on the first beat.
pub struct GuitarTunaComParser;

impl SongParser for GuitarTunaComParser {
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

        let composition = composition(&document);
        let field = |path: &str| -> Option<String> {
            composition
                .as_ref()?
                .pointer(path)?
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let summary = field("/text").unwrap_or_default();

        let sheet = sheet(&document);
        if sheet.is_empty() {
            eprintln!("Warning: no song found on {}.", url);
        }

        let meter = Selector::parse("#songcontent [data-time]")
            .ok()
            .and_then(|selector| document.select(&selector).next())
            .and_then(|el| el.value().attr("data-time"))
            .and_then(meter_of);

        // The tempo is printed as "173 BPM"; the number is what is kept.
        let tempo = labelled(&summary, "Tempo")
            .map(|tempo| {
                tempo
                    .trim_end_matches(|c: char| !c.is_ascii_digit())
                    .trim()
                    .to_string()
            })
            .filter(|tempo| !tempo.is_empty());

        // Standard tuning is what a song without one is played in anyway.
        let tuning = labelled(&summary, "Tuning").filter(|tuning| {
            tuning.split_whitespace().collect::<Vec<_>>() != ["E", "A", "D", "G", "B", "E"]
        });

        let capo = labelled(&summary, "Capo")
            .and_then(|fret| fret.trim().parse::<u8>().ok())
            .filter(|fret| *fret > 0);

        Song {
            title: text_of("h1[data-testid=song-title] > span:first-child")
                .or_else(|| {
                    field("/name").map(|name| name.trim_end_matches("(chords)").trim().to_string())
                })
                .unwrap_or_else(|| "Unknown Title".to_string()),
            artist: text_of("h1[data-testid=song-title] a")
                .or_else(|| field("/composer/name"))
                .unwrap_or_else(|| "Unknown Artist".to_string()),
            source_website: url.to_string(),
            key: field("/musicalKey")
                .or_else(|| labelled(&summary, "Key"))
                .map(|key| key_name(&key)),
            tempo,
            meter,
            language: None, // Not provided by the site.
            tuning,
            capo,
            extra: Vec::new(),
            parts: parse_body(&sheet),
        }
    }
}

/// The `MusicComposition` the page describes itself with for search engines.
fn composition(document: &Html) -> Option<Value> {
    let selector = Selector::parse(r#"script[type="application/ld+json"]"#).ok()?;
    document
        .select(&selector)
        .filter_map(|el| serde_json::from_str::<Value>(&el.text().collect::<String>()).ok())
        .find(|data| data.get("@type").and_then(Value::as_str) == Some("MusicComposition"))
}

/// The value behind `label:` in the one-line summary the JSON-LD carries —
/// "Tuning: E A D G B E Key: Eb major Tempo: 173 BPM" — which runs up to the
/// next label.
fn labelled(summary: &str, label: &str) -> Option<String> {
    const LABELS: [&str; 4] = ["Tuning:", "Key:", "Tempo:", "Capo:"];

    let start = summary.find(&format!("{}:", label))? + label.len() + 1;
    let rest = &summary[start..];
    let end = LABELS
        .iter()
        .filter_map(|next| rest.find(next))
        .min()
        .unwrap_or(rest.len());
    Some(rest[..end].trim().to_string()).filter(|value| !value.is_empty())
}

/// The key the way a chord names it: "Eb major" is "Eb", "C minor" is "Cm".
fn key_name(key: &str) -> String {
    let mut words = key.split_whitespace();
    let root = words.next().unwrap_or_default();
    match words.next().map(str::to_lowercase).as_deref() {
        Some("minor") => format!("{}m", root),
        Some("major") | None => root.to_string(),
        Some(_) => shorten_minor_name(key.trim()),
    }
}

/// The song written out as "chords over lyrics", a `[Section]` heading every
/// part and a blank line between them.
fn sheet(document: &Html) -> String {
    let Ok(selector) = Selector::parse("#songcontent .part_type, #songcontent .line") else {
        return String::new();
    };

    let mut sheet = Vec::new();
    for el in document.select(&selector) {
        if el.value().classes().any(|class| class == "part_type") {
            if !sheet.is_empty() {
                sheet.push(String::new());
            }
            let name = el.text().collect::<String>();
            sheet.push(format!("[{}]", name.trim()));
        } else {
            sheet.extend(line(el));
        }
    }
    sheet.join("\n")
}

/// One line of the song: the chords over it, if it has any, then its lyrics,
/// if it has any.
fn line(el: ElementRef) -> Vec<String> {
    let chords: Vec<String> = Selector::parse("[data-chord]")
        .map(|selector| {
            el.select(&selector)
                .filter_map(|beat| beat.value().attr("data-chord"))
                .map(str::trim)
                .filter(|chord| !chord.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut lyrics = String::new();
    let mut marks = Vec::new();
    if let Some(words) = el
        .children()
        .filter_map(ElementRef::wrap)
        .find(|child| child.value().name() == "div")
    {
        read_lyrics(words, &mut lyrics, &mut marks);
    }
    let lyrics = lyrics.trim_end().to_string();

    // Chords played on the same syllable — or past the end of the lyrics, as
    // in a line with none — are set one after another, a space apart.
    let mut over = String::new();
    for (i, chord) in chords.iter().enumerate() {
        let width = over.chars().count();
        let column = marks
            .get(i)
            .copied()
            .unwrap_or(0)
            .max(if width == 0 { 0 } else { width + 1 });
        over.extend(std::iter::repeat_n(' ', column - width));
        over.push_str(chord);
    }

    [over, lyrics]
        .into_iter()
        .filter(|row| !row.trim().is_empty())
        .collect()
}

/// Gathers the lyrics of a line, noting the column at which each chord the
/// screen reader is told about stands.
fn read_lyrics(el: ElementRef, lyrics: &mut String, marks: &mut Vec<usize>) {
    for child in el.children() {
        match child.value() {
            // The site pads the lyrics out to the width of the beats they
            // span; a run of spaces is one space in the song.
            Node::Text(text) => {
                for c in text
                    .chars()
                    .map(|c| if c.is_whitespace() { ' ' } else { c })
                {
                    if !(c == ' ' && lyrics.ends_with(' ')) {
                        lyrics.push(c);
                    }
                }
            }
            Node::Element(element) if element.classes().any(|class| class == "sr-only") => {
                marks.push(lyrics.chars().count());
            }
            Node::Element(_) => {
                if let Some(child) = ElementRef::wrap(child) {
                    read_lyrics(child, lyrics, marks);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Line, PartType};

    /// A beat of a line, carrying a chord or not.
    fn beat(chord: Option<&str>) -> String {
        match chord {
            Some(chord) => format!(
                "<span><div class=\"beat_info\" data-pos=\"0:0.0\"><div>\
                 <span data-chord=\"{0} \" aria-hidden=\"true\">{0} </span></div></div></span>",
                chord
            ),
            None => "<span><div class=\"beat_info\" data-pos=\"0:0.840\"></div></span>".to_string(),
        }
    }

    /// A line as the site writes it: `|` in the lyrics stands where the next
    /// chord is played.
    fn line(chords: &[&str], lyrics: &str) -> String {
        let beats: String = chords
            .iter()
            .flat_map(|chord| [beat(Some(chord)), beat(None)])
            .collect();
        let lyrics = lyrics.replace('|', "<span class=\"sr-only\"> (a chord) </span>");
        format!(
            "<div class=\"line\" data-bar=\"0\">{}<span></span><div>{}</div><div></div></div>",
            beats, lyrics
        )
    }

    fn part(name: &str, lines: &[String]) -> String {
        format!(
            "<div><div class=\"partContainer\"><div class=\"part_type\">{}</div>\
             <span class=\"partDivider\"></span></div>{}</div>",
            name,
            lines.concat()
        )
    }

    fn page(parts: &[String]) -> String {
        format!(
            "<html><head><script type=\"application/ld+json\">{{\"@context\":\"https://schema.org\",\
             \"@type\":\"MusicComposition\",\"name\":\"A Song (chords)\",\
             \"composer\":{{\"@type\":\"Person\",\"name\":\"A Band\"}},\"musicalKey\":\"Eb major\",\
             \"text\":\"Tuning: E A D G B E Key: Eb major Tempo: 173 BPM\"}}</script></head><body>\
             <h1 data-testid=\"song-title\"><span class=\"font-bold uppercase\">A Song</span> \
             <span>chords by</span> <span><a href=\"/artists/a-band\">A Band</a></span></h1>\
             <div id=\"songcontent\" data-testid=\"song-lyrics\">{}</div></body></html>",
            parts
                .concat()
                .replacen("data-pos", "data-time=\"3/4\" data-pos", 1)
        )
    }

    fn parse(parts: &[String]) -> Song {
        GuitarTunaComParser.parse("https://guitartuna.com/chords/a-song", &page(parts))
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
    fn reads_the_song_and_what_is_printed_above_it() {
        let song = parse(&[part("Verse 1", &[line(&["Bb"], "She can |kill")])]);

        assert_eq!(song.title, "A Song");
        assert_eq!(song.artist, "A Band");
        assert_eq!(song.key.as_deref(), Some("Eb"));
        assert_eq!(song.tempo.as_deref(), Some("173"));
        assert_eq!(song.meter.as_deref(), Some("3/4"));
        assert_eq!(song.tuning, None);
        assert_eq!(song.capo, None);
    }

    #[test]
    fn a_chord_stands_where_the_lyrics_mark_it() {
        let song = parse(&[part(
            "Verse 1",
            &[
                line(&["Bb", "Eb"], "She can |kill with a smile, |"),
                line(&["Ab"], "And she can ruin your |faith"),
            ],
        )]);

        let verse = &song.parts[0];
        assert!(matches!(verse.part_type, PartType::Verse));
        assert_eq!(verse.lines[0].lyrics, "She can kill with a smile,");
        assert_eq!(
            chords(&verse.lines[0]),
            [("Bb".to_string(), 8), ("Eb".to_string(), 27)]
        );
        assert_eq!(verse.lines[1].lyrics, "And she can ruin your faith");
        assert_eq!(chords(&verse.lines[1]), [("Ab".to_string(), 22)]);
    }

    /// Two chords on one syllable cannot share its column.
    #[test]
    fn chords_on_one_syllable_are_set_side_by_side() {
        let song = parse(&[part(
            "Verse 1",
            &[line(
                &["Abmaj7", "Ab6", "Abmaj7"],
                "with her |casual    lies ||",
            )],
        )]);

        assert_eq!(
            chords(&song.parts[0].lines[0]),
            [
                ("Abmaj7".to_string(), 9),
                ("Ab6".to_string(), 21),
                ("Abmaj7".to_string(), 25),
            ]
        );
    }

    #[test]
    fn a_part_of_chords_alone_is_read_as_one() {
        let song = parse(&[
            part("Intro 1", &[line(&["Eb", "Ebsus4", "Eb", "Bb"], "||||")]),
            part("Verse 1", &[line(&["Bb"], "She can |kill")]),
        ]);

        assert_eq!(song.parts.len(), 2);
        assert!(matches!(song.parts[0].part_type, PartType::Intro));
        assert_eq!(
            chords(&song.parts[0].lines[0]),
            [
                ("Eb".to_string(), 0),
                ("Ebsus4".to_string(), 3),
                ("Eb".to_string(), 10),
                ("Bb".to_string(), 13),
            ]
        );
        assert!(matches!(song.parts[1].part_type, PartType::Verse));
    }

    #[test]
    fn the_key_is_named_the_way_a_chord_is() {
        assert_eq!(key_name("Eb major"), "Eb");
        assert_eq!(key_name("C# minor"), "C#m");
        assert_eq!(key_name("A"), "A");
    }

    #[test]
    fn a_label_reaches_up_to_the_next_one() {
        let summary = "Tuning: D A D G B E Key: C minor Tempo: 96 BPM Capo: 2";
        assert_eq!(labelled(summary, "Tuning").as_deref(), Some("D A D G B E"));
        assert_eq!(labelled(summary, "Key").as_deref(), Some("C minor"));
        assert_eq!(labelled(summary, "Capo").as_deref(), Some("2"));
        assert_eq!(labelled(summary, "Year"), None);
    }

    #[test]
    fn a_page_carrying_no_song_still_parses() {
        let song = GuitarTunaComParser.parse("https://guitartuna.com/", "<html></html>");

        assert_eq!(song.title, "Unknown Title");
        assert!(song.parts.is_empty());
    }
}
