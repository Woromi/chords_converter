use super::SongParser;
use crate::models::{Chord, Line, Part, PartType, PlacedChord, Song};

/// Parses plain-text songs written in the common "chords over lyrics" layout,
/// e.g. a line of chords positioned above the lyric line they belong to:
///
/// ```text
/// Beatles - Let It Be
/// ===================
/// Key:   C
/// Capo:  0
///
/// [Verse]
///        C          G
/// When I find myself in times of trouble
/// ```
///
/// The header line, the `Label: value` metadata block and the section markers
/// (`[Verse]`, `R:`, `1.`, ...) are all optional. Anything that is not a chord
/// line, marker or metadata is treated as lyrics.
pub struct TextParser;

/// Column width used when expanding tabs. A chord line is aligned to the lyric
/// line below it, so both have to agree on where a tab lands.
const TAB_WIDTH: usize = 8;

impl SongParser for TextParser {
    fn parse(&self, source: &str, content: &str) -> Song {
        let content = content.trim_start_matches('\u{feff}');
        let lines: Vec<String> = content.lines().map(expand_tabs).collect();

        let mut header = Header::new(source);
        let idx = parse_header(&lines, &mut header);

        header.into_song(parse_parts(lines, idx))
    }
}

/// Parses a whole "chords over lyrics" body — no header, no metadata — into
/// parts. The website parsers reach for it once they have stripped their own
/// markup, their pages using the very same layout underneath.
pub(super) fn parse_body(content: &str) -> Vec<Part> {
    parse_parts(content.lines().map(expand_tabs).collect(), 0)
}

/// Walks the body lines from `idx` on, attaching each chord line to the lyric
/// line directly beneath it.
fn parse_parts(mut lines: Vec<String>, mut idx: usize) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    let mut current_type = PartType::Verse;
    // Whether a marker said so, or the part is simply the one we default to.
    let mut named = false;
    let mut current_lines: Vec<Line> = Vec::new();

    while idx < lines.len() {
        let line = lines[idx].clone();
        let line = line.as_str();
        let trimmed = line.trim();

        // A. Blank line: closes the current block, keeps the section type
        //    so that a multi-stanza chorus stays a chorus.
        if trimmed.is_empty() {
            flush_part(&mut parts, &current_type, named, &mut current_lines);
            idx += 1;
            continue;
        }

        // B. Bracketed section marker, e.g. "[Chorus]" or "[Verse 2]".
        if let Some(pt) = parse_bracket_marker(trimmed) {
            flush_part(&mut parts, &current_type, named, &mut current_lines);
            current_type = pt;
            named = true;
            idx += 1;
            continue;
        }

        // C. Inline structural marker, e.g. "R:", "1." (Czech/English).
        //    The marker is blanked out rather than cut off — "Intro: | C G |",
        //    "R:  C      G" and "1. When I find myself" all keep the columns
        //    they were written in, so the chords still stand over the right
        //    syllables and the lines of a part still line up with each other.
        if let Some((pt, rest_at)) = detect_part_type(line) {
            flush_part(&mut parts, &current_type, named, &mut current_lines);
            current_type = pt;
            named = true;
            lines[idx] = blank_out(line, rest_at);
            continue;
        }

        // D. Tablature: a run of string lines, taken verbatim into a part of
        //    its own. Pairing it with the chords above it as though it were
        //    lyrics would write them into the middle of the staff.
        if is_tablature_line(line) {
            flush_part(&mut parts, &current_type, named, &mut current_lines);

            let mut staff = Vec::new();
            while idx < lines.len() && is_tablature_line(&lines[idx]) {
                staff.push(Line {
                    lyrics: lines[idx].trim_end().to_string(),
                    chords: Vec::new(),
                });
                idx += 1;
            }
            parts.push(Part {
                part_type: PartType::Tab,
                lines: dedent(staff),
            });
            continue;
        }

        // E. Chord line: attach it to the lyric line beneath, or keep it as
        //    an instrumental (chords-only) line when no lyrics follow.
        if is_chord_line(line) {
            let chords = parse_chord_line(line);

            // The lyric line below may carry a marker, as in "1. When I find
            // myself ...". It opens a part the chords above it already belong
            // to, so the part is closed here, before they are attached.
            let mut next = lines.get(idx + 1).cloned().unwrap_or_default();
            if let Some((pt, rest_at)) = detect_part_type(&next) {
                flush_part(&mut parts, &current_type, named, &mut current_lines);
                current_type = pt;
                named = true;
                next = blank_out(&next, rest_at);
                lines[idx + 1] = next.clone();
            }

            let next_trim = next.trim();
            let follows_lyrics = !next_trim.is_empty()
                && !is_chord_line(&next)
                && !is_tablature_line(&next)
                && parse_bracket_marker(next_trim).is_none();

            if !follows_lyrics {
                current_lines.push(Line {
                    lyrics: decoration_skeleton(line),
                    chords,
                });
                idx += 1;
                continue;
            }

            current_lines.push(Line {
                lyrics: next.trim_end().to_string(),
                chords,
            });
            idx += 2;
            continue;
        }

        // F. Plain lyric line with no chords above it.
        current_lines.push(Line {
            lyrics: line.trim_end().to_string(),
            chords: Vec::new(),
        });
        idx += 1;
    }

    flush_part(&mut parts, &current_type, named, &mut current_lines);

    parts
}

/// Metadata gathered from the header, before the body is parsed.
struct Header {
    title: String,
    artist: String,
    source_website: String,
    key: Option<String>,
    tempo: Option<String>,
    meter: Option<String>,
    tuning: Option<String>,
    language: Option<String>,
    capo: Option<u8>,
    extra: Vec<(String, String)>,
}

impl Header {
    fn new(source: &str) -> Self {
        Header {
            title: "Unknown Title".to_string(),
            artist: "Unknown Artist".to_string(),
            source_website: source.to_string(),
            key: None,
            tempo: None,
            meter: None,
            tuning: None,
            language: None,
            capo: None,
            extra: Vec::new(),
        }
    }

    /// Stores a `Label: value` pair, keeping labels we have no field for.
    fn apply(&mut self, label: &str, value: &str) {
        let value = value.to_string();
        match label.to_lowercase().as_str() {
            "title" | "song" | "název" | "nazev" | "píseň" | "pisen" => self.title = value,
            "artist" | "performer" | "band" | "interpret" | "kapela" | "skupina" => {
                self.artist = value
            }
            "key" | "tónina" | "tonina" => self.key = Some(shorten_minor_name(&value)),
            "tempo" | "bpm" => self.tempo = Some(value),
            "meter" | "time" | "time signature" | "metrum" | "takt" | "taktové označení" => {
                self.meter = meter_of(&value)
            }
            "tuning" | "ladění" | "ladeni" => self.tuning = Some(value),
            "language" | "lang" | "jazyk" => self.language = Some(value),
            "capo" | "kapo" | "capodastr" => {
                self.capo = value.split_whitespace().next().and_then(|v| v.parse().ok())
            }
            "source" | "zdroj" | "url" => self.source_website = value,
            _ => self.extra.push((label.to_string(), value)),
        }
    }

    fn into_song(self, parts: Vec<Part>) -> Song {
        Song {
            title: self.title,
            artist: self.artist,
            source_website: self.source_website,
            key: self.key,
            tempo: self.tempo,
            meter: self.meter,
            language: self.language,
            tuning: self.tuning,
            capo: self.capo,
            extra: self.extra,
            parts,
        }
    }
}

/// Reads a time signature such as `3/4` or `6 / 8` out of a label's value.
pub fn meter_of(value: &str) -> Option<String> {
    let (num, den) = value.split_once('/')?;
    let num = num.trim().rsplit(|c: char| !c.is_ascii_digit()).next()?;
    let den = den.trim().split(|c: char| !c.is_ascii_digit()).next()?;
    (!num.is_empty() && !den.is_empty()).then(|| format!("{}/{}", num, den))
}

/// Consumes the optional header — an `Artist - Title` line followed either by
/// an `====` underline or by metadata, plus the `Label: value` block below it —
/// and returns the index of the first body line.
fn parse_header(lines: &[String], header: &mut Header) -> usize {
    let mut idx = 0;
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }

    let title_line = lines.get(idx).map(|l| l.trim()).unwrap_or("");
    let below = lines.get(idx + 1).map(|l| l.trim()).unwrap_or("");
    let underlined = !below.is_empty() && below.chars().all(|c| c == '=' || c == '-');

    if !title_line.is_empty()
        && (underlined || is_metadata(below))
        && !is_metadata(title_line)
        && parse_bracket_marker(title_line).is_none()
        && detect_part_type(title_line).is_none()
        && !is_chord_line(title_line)
    {
        match title_line.split_once(" - ") {
            Some((artist, title)) => {
                header.artist = artist.trim().to_string();
                header.title = title.trim().to_string();
            }
            None => header.title = title_line.to_string(),
        }
        idx += if underlined { 2 } else { 1 };
    }

    // Metadata block: contiguous "Label: value" lines, terminated by a blank
    // line or by the first line that is not metadata.
    while idx < lines.len() {
        let line = lines[idx].trim();
        if line.is_empty() {
            idx += 1;
            break;
        }
        if parse_bracket_marker(line).is_some() || detect_part_type(line).is_some() {
            break;
        }
        match parse_metadata(line) {
            Some((label, value)) => {
                header.apply(label, value);
                idx += 1;
            }
            None => break,
        }
    }

    idx
}

/// Standard and Czech chord roots, ordered so multi-character roots ("Cis",
/// "C#", "Cb") are matched before the single-character ones ("C").
const ROOTS: [&str; 31] = [
    "Cis", "Dis", "Fis", "Gis", "Ais", "Ces", "Des", "Fes", "Ges", "C#", "D#", "F#", "G#", "A#",
    "Cb", "Db", "Eb", "Fb", "Gb", "Ab", "Bb", "Es", "As", "C", "D", "E", "F", "G", "A", "H", "B",
];

/// Section markers recognised at the start of a line, longest keyword first so
/// that "refren" wins over "ref".
const MARKERS: [(&str, PartMarker); 22] = [
    ("instrumental", PartMarker::Instrumental),
    ("prechorus", PartMarker::PreChorus),
    ("pre-chorus", PartMarker::PreChorus),
    ("interlude", PartMarker::Instrumental),
    ("předehra", PartMarker::Intro),
    ("predehra", PartMarker::Intro),
    ("refrain", PartMarker::Chorus),
    ("refrén", PartMarker::Chorus),
    ("refren", PartMarker::Chorus),
    ("mezihra", PartMarker::Bridge),
    ("chorus", PartMarker::Chorus),
    ("bridge", PartMarker::Bridge),
    ("ending", PartMarker::Outro),
    ("dohra", PartMarker::Outro),
    ("intro", PartMarker::Intro),
    ("outro", PartMarker::Outro),
    ("sloka", PartMarker::Verse),
    ("verse", PartMarker::Verse),
    ("solo", PartMarker::Instrumental),
    ("ref", PartMarker::Chorus),
    ("tab", PartMarker::Tab),
    ("r", PartMarker::Chorus),
];

/// The part types a keyword marker can select (a subset of `PartType` without
/// the free-form `Other` variant, so the table above can be a `const`).
#[derive(Clone, Copy)]
enum PartMarker {
    Intro,
    Verse,
    PreChorus,
    Chorus,
    Bridge,
    Instrumental,
    Tab,
    Outro,
}

impl PartMarker {
    fn part_type(self) -> PartType {
        match self {
            PartMarker::Intro => PartType::Intro,
            PartMarker::Verse => PartType::Verse,
            PartMarker::PreChorus => PartType::PreChorus,
            PartMarker::Chorus => PartType::Chorus,
            PartMarker::Bridge => PartType::Bridge,
            PartMarker::Instrumental => PartType::Instrumental,
            PartMarker::Tab => PartType::Tab,
            PartMarker::Outro => PartType::Outro,
        }
    }
}

/// Expands tabs to the next tab stop so that chord columns keep matching the
/// lyric columns even when the source mixes tabs and spaces.
fn expand_tabs(line: &str) -> String {
    if !line.contains('\t') {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + TAB_WIDTH);
    let mut col = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let width = TAB_WIDTH - (col % TAB_WIDTH);
            out.extend(std::iter::repeat_n(' ', width));
            col += width;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

/// Returns the root that `s` starts with, if any.
fn match_root(s: &str) -> Option<&'static str> {
    ROOTS.iter().copied().find(|r| s.starts_with(r))
}

/// Replaces the first `upto` bytes of a line with as many spaces, which keeps
/// what follows in the column it was written in.
fn blank_out(line: &str, upto: usize) -> String {
    " ".repeat(line[..upto].chars().count()) + &line[upto..]
}

/// Pushes the accumulated lines as a finished part (no-op when empty).
///
/// A block no marker named, in which not a word is sung, is not the verse we
/// default to: it is the intro when it opens the song and an instrumental
/// wherever else it stands.
fn flush_part(
    parts: &mut Vec<Part>,
    current_type: &PartType,
    named: bool,
    current_lines: &mut Vec<Line>,
) {
    if current_lines.is_empty() {
        return;
    }

    let unsung = current_lines.iter().all(is_unsung);
    let part_type = match !named && unsung {
        true if parts.is_empty() => PartType::Intro,
        true => PartType::Instrumental,
        false => current_type.clone(),
    };

    parts.push(Part {
        part_type,
        lines: dedent(std::mem::take(current_lines)),
    });
}

/// A line with nothing to sing — bar lines and repeat marks are all that is
/// left of it once the chords are taken out.
fn is_unsung(line: &Line) -> bool {
    tokenize(&line.lyrics)
        .into_iter()
        .all(|(_, token)| is_decoration(token))
}

/// Drops the indentation a part shares. Sheets are written with the song
/// indented — under a "1." that only the first line carries, or simply because
/// the site drew it that way — and carrying that margin into the songbook
/// would only offset every line of it. Chords and lyrics move together, so
/// what stood above what still does.
fn dedent(lines: Vec<Line>) -> Vec<Line> {
    let margin = lines
        .iter()
        .filter_map(|line| {
            let written = !line.lyrics.trim().is_empty();
            let text = written.then(|| line.lyrics.chars().take_while(|c| *c == ' ').count());
            let chord = line.chords.first().map(|placed| placed.position);
            match (text, chord) {
                (Some(text), Some(chord)) => Some(text.min(chord)),
                (text, chord) => text.or(chord),
            }
        })
        .min()
        .unwrap_or(0);

    if margin == 0 {
        return lines;
    }

    lines
        .into_iter()
        .map(|line| Line {
            lyrics: line.lyrics.chars().skip(margin).collect(),
            chords: line
                .chords
                .into_iter()
                .map(|placed| PlacedChord {
                    position: placed.position.saturating_sub(margin),
                    ..placed
                })
                .collect(),
        })
        .collect()
}

/// Recognises a `Label: value` metadata line. Any short, alphabetic label is
/// accepted; unknown ones are kept verbatim in `Song::extra`.
fn parse_metadata(line: &str) -> Option<(&str, &str)> {
    let (label, value) = line.split_once(':')?;
    let label = label.trim();
    let value = value.trim();
    let plausible = !value.is_empty()
        && label.chars().count() <= 24
        && label.chars().any(char::is_alphabetic)
        && label
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '.' | '&' | '/'));
    plausible.then_some((label, value))
}

fn is_metadata(line: &str) -> bool {
    parse_metadata(line).is_some()
}

/// Maps a `[Bracketed]` marker to a part type, ignoring any trailing number.
fn parse_bracket_marker(line: &str) -> Option<PartType> {
    let trimmed = line.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return None;
    }
    // "[Intro:]" names the same section as "[Intro]".
    let inner = trimmed[1..trimmed.len() - 1]
        .trim()
        .trim_end_matches(':')
        .trim()
        .to_lowercase();
    let base = inner
        .split(|c: char| c.is_whitespace() || c.is_ascii_digit())
        .next()
        .unwrap_or("");
    let pt = match lookup_marker(base) {
        Some(marker) => marker.part_type(),
        None => PartType::Other(inner),
    };
    Some(pt)
}

fn lookup_marker(keyword: &str) -> Option<PartMarker> {
    MARKERS
        .iter()
        .find(|(name, _)| *name == keyword)
        .map(|(_, marker)| *marker)
}

/// Identifies a structural marker at the start of a line — a keyword ("R",
/// "Ref", "Chorus", "Sloka 2") or a plain number ("1.", "2)") followed by `:`,
/// `.`, `)` or the end of the line. A marker may also be written in brackets
/// of its own, "(1)", "(R)" or "(Ref:)", as Czech songbooks tend to.
///
/// Returns the part type together with the byte offset at which the rest of
/// the line (if any) continues, so that "1. Some lyrics" keeps its lyrics.
fn detect_part_type(line: &str) -> Option<(PartType, usize)> {
    let indent = line.len() - line.trim_start().len();
    let opened = &line[indent..];

    // A bracketed marker has to close its bracket, which is what keeps a
    // parenthesis the lyrics open — "(ona šla domů)" — from becoming one.
    let bracketed = opened.starts_with('(');
    let body = if bracketed { &opened[1..] } else { opened };

    // Leading alphabetic run: the keyword.
    let keyword_len = body
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic())
        .map_or(body.len(), |(i, _)| i);
    let keyword = body[..keyword_len].to_lowercase();
    let mut cursor = keyword_len;

    // Optional section number: "Verse 2", "Sloka 3", or a bare "1.".
    let spaces = body[cursor..].len() - body[cursor..].trim_start_matches(' ').len();
    cursor += spaces;
    let digits = body[cursor..]
        .chars()
        .take_while(char::is_ascii_digit)
        .count();
    if digits > 2 {
        return None;
    }
    cursor += digits;

    let separator = body[cursor..].chars().next();
    if bracketed {
        let after = match separator {
            Some(':') | Some('.') => &body[cursor + 1..],
            _ => &body[cursor..],
        };
        let closed = after.strip_prefix(')')?;
        cursor = body.len() - closed.len();
    } else {
        match separator {
            Some(':') | Some('.') | Some(')') => cursor += 1,
            // A marker may also stand alone on its line, e.g. "Intro".
            None => {}
            _ => return None,
        }
    }

    let marker = if keyword.is_empty() {
        // A bare number is only a marker when it is numbered and separated,
        // "1." starts a verse but "1973" is just text.
        if digits == 0 || (!bracketed && separator.is_none()) {
            return None;
        }
        PartMarker::Verse
    } else {
        match lookup_marker(&keyword) {
            Some(marker) => marker,
            None if bracketed => bracketed_marker(&keyword)?,
            None => return None,
        }
    };

    Some((marker.part_type(), indent + usize::from(bracketed) + cursor))
}

/// The one-letter markers that are only readable inside brackets. On their own
/// "S" and "I" are far more likely to be a word of the lyrics than a section.
fn bracketed_marker(keyword: &str) -> Option<PartMarker> {
    match keyword {
        "s" => Some(PartMarker::Verse),
        "i" => Some(PartMarker::Intro),
        _ => None,
    }
}

/// A line is a chord line when every token is a chord or a decoration
/// (`|`, `x2`, ...) and at least one of them is a real chord.
fn is_chord_line(line: &str) -> bool {
    let mut chords = 0;
    for (_, token) in tokenize(without_trailing_remark(line)) {
        if chord_token(token).is_some() {
            chords += 1;
        } else if !is_decoration(token) {
            return false;
        }
    }
    chords > 0
}

/// Splits a line into `(column, token)` pairs, the column being the character
/// offset the token starts at.
fn tokenize(line: &str) -> Vec<(usize, &str)> {
    let mut tokens = Vec::new();
    let mut start: Option<(usize, usize)> = None;

    for (col, (byte, ch)) in line.char_indices().enumerate() {
        if ch.is_whitespace() {
            if let Some((col_start, byte_start)) = start.take() {
                tokens.push((col_start, &line[byte_start..byte]));
            }
        } else if start.is_none() {
            start = Some((col, byte));
        }
    }
    if let Some((col_start, byte_start)) = start {
        tokens.push((col_start, &line[byte_start..]));
    }

    tokens
}

/// Blanks the chords out of a chord line, keeping bar lines and repeat marks
/// in their original columns. An instrumental line uses the result as its
/// "lyrics" so that `| C  G | Am  F |` does not lose its bars.
fn decoration_skeleton(line: &str) -> String {
    let mut chars: Vec<char> = line.chars().collect();
    for (col, token) in tokenize(without_trailing_remark(line)) {
        if let Some(chord) = chord_token(token) {
            let dropped = chord.as_ptr() as usize - token.as_ptr() as usize;
            let start = col + token[..dropped].chars().count();
            // The chord takes the punctuation it was written with — the commas
            // of "C, G, Am" — away with it. Left behind, they would read as a
            // line of lyrics made of stray commas.
            let tail = &token[dropped + chord.len()..];
            let punctuation = tail.chars().all(|c| matches!(c, ',' | ';' | '\'' | '"'));
            let width = chord.chars().count() + if punctuation { tail.chars().count() } else { 0 };
            for c in chars.iter_mut().skip(start).take(width) {
                *c = ' ';
            }
        } else if token.chars().all(|c| matches!(c, ',' | ';')) {
            // A comma the chords were strung together with, left on its own
            // by the spacing of the line.
            for c in chars.iter_mut().skip(col).take(token.chars().count()) {
                *c = ' ';
            }
        }
    }
    chars.into_iter().collect::<String>().trim_end().to_string()
}

/// Unwraps a token from its decoration and returns the bare chord when the
/// token is one. The result is a slice of `token`, so the caller can still tell
/// which column the chord itself starts in.
fn chord_token(token: &str) -> Option<&str> {
    let mut chord = token.trim_end_matches([',', ';', '\'', '"']);

    // A chord in brackets of its own: "(C)".
    if chord.len() > 2
        && chord.starts_with('(')
        && chord.ends_with(')')
        && is_chord_token(&chord[1..chord.len() - 1])
    {
        chord = &chord[1..chord.len() - 1];
    }
    // Brackets around a whole run of chords, opening and closing on different
    // tokens: "G (C B7) D". What is left over on this token is not its own.
    while chord.matches(')').count() > chord.matches('(').count() {
        chord = chord.strip_suffix(')')?;
    }
    while chord.matches('(').count() > chord.matches(')').count() {
        chord = chord.strip_prefix('(')?;
    }

    (!chord.is_empty() && is_chord_token(chord)).then_some(chord)
}

/// Recognises one string of an ASCII tablature staff — "e|--7--|--9--|". The
/// string it is named after may be there or not, but the bar lines and the
/// frets strung between them have to be.
fn is_tablature_line(line: &str) -> bool {
    const FRET_MARKS: &str = "hpbrsxtv/\\~*() .";

    let trimmed = line.trim();
    let body = match trimmed.split_once('|') {
        Some((string, body)) if string.chars().count() <= 2 => body,
        _ => return false,
    };

    body.chars().filter(|c| *c == '-').count() >= 4
        && body
            .chars()
            .all(|c| c == '-' || c == '|' || c.is_ascii_digit() || FRET_MARKS.contains(c))
}

/// Cuts off the aside whoever wrote the tab may have left at the end of a
/// chord line — "(not sure about this one)". Brackets holding nothing but
/// chords are left alone: there they group a run of chords, as in "G (C B7) D".
fn without_trailing_remark(line: &str) -> &str {
    let trimmed = line.trim_end();
    let open = match trimmed.strip_suffix(')').and_then(|_| trimmed.rfind('(')) {
        Some(open) => open,
        None => return line,
    };

    // The bracket has to open a group of its own. One glued to the token in
    // front of it belongs to that token, the way the "(7)" of "B(7)" does.
    if open > 0 && !trimmed[..open].ends_with(char::is_whitespace) {
        return line;
    }

    let inside = &trimmed[open + 1..trimmed.len() - 1];
    let reads_as_chords = tokenize(inside)
        .into_iter()
        .all(|(_, token)| chord_token(token).is_some() || is_decoration(token));

    if reads_as_chords { line } else { &line[..open] }
}

/// Bar lines, repeat marks and repeat counts, all of which may share a line
/// with chords without making it a lyric line.
fn is_decoration(token: &str) -> bool {
    const MARKS: [char; 10] = ['|', ':', '-', '/', '\\', '%', '*', '.', ',', '='];

    let token = token.trim_matches(['(', ')', '[', ']']);
    if token.is_empty() {
        return true;
    }
    if token.chars().all(|c| MARKS.contains(&c)) {
        return true;
    }
    // Repeat counts: "x2", "2x", "3×".
    let lower = token.to_lowercase();
    let digits = lower.trim_start_matches('x').trim_end_matches(['x', '×']);
    digits.len() < lower.len() && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// A chord token is a chord body optionally followed by a `/bass` note. Both
/// `C/E` and the `C\E` spelling some songbooks use are accepted.
fn is_chord_token(token: &str) -> bool {
    if is_no_chord_mark(token) {
        return true;
    }

    let mut segments = token.split(is_bass_separator);
    match segments.next() {
        Some(main) if is_chord_body(main) => {}
        _ => return false,
    }
    // Every `/bass` segment must itself read as a note.
    segments.all(|seg| !seg.is_empty() && is_chord_body(seg))
}

fn is_bass_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Recognises the "no chord" mark. Whoever writes a sheet writes it with as
/// many of its dots as they feel like, and now and then with a slash where a
/// dot should be — which is why it is read off the whole token, before the
/// `/bass` of a chord is looked for in it.
fn is_no_chord_mark(token: &str) -> bool {
    const SPELLINGS: [&str; 4] = ["n.c.", "n.c", "n/c", "nc"];

    SPELLINGS.contains(&token.to_lowercase().as_str())
}

/// A chord body is a known root followed only by chord-quality characters.
fn is_chord_body(s: &str) -> bool {
    match match_root(s) {
        Some(root) => s[root.len()..].chars().all(is_quality_char),
        None => false,
    }
}

/// Characters allowed in a chord quality (after the root), covering suffixes
/// like `maj7`, `mi`, `sus4`, `add9`, `dim`, `aug`, `7b5`, `#9`.
fn is_quality_char(c: char) -> bool {
    c.is_ascii_digit()
        || matches!(
            c,
            'a' | 'd'
                | 'g'
                | 'i'
                | 'j'
                | 'm'
                | 'n'
                | 'o'
                | 's'
                | 'u'
                | 'M'
                | '#'
                | 'b'
                | '+'
                | '-'
                | '('
                | ')'
                | '.'
                | '*'
                | '°'
                | 'Δ'
                | 'ø'
                | '∅'
        )
}

/// Extracts chords and their character column from a chord line.
fn parse_chord_line(line: &str) -> Vec<PlacedChord> {
    tokenize(without_trailing_remark(line))
        .into_iter()
        .filter_map(|(position, token)| {
            let chord = chord_token(token)?;
            // The token may open with a bracket the chord does not own, so
            // count what was dropped to keep the column exact.
            let dropped = chord.as_ptr() as usize - token.as_ptr() as usize;
            Some(PlacedChord {
                chord: parse_chord(chord),
                position: position + token[..dropped].chars().count(),
            })
        })
        .collect()
}

/// Splits a raw chord string like "D/F#", "C\E" or "C#mi7" into a `Chord`.
/// The website parsers reach for it wherever their page hands them a chord
/// as one string, so that "Hmi7/Fis" is read apart the same way everywhere.
pub(super) fn parse_chord(s: &str) -> Chord {
    if is_no_chord_mark(s.trim()) {
        return Chord::no_chord();
    }

    let (main_part, bass) = match s.find(is_bass_separator) {
        Some(pos) => {
            let bass = s[pos + 1..].trim();
            (
                s[..pos].trim(),
                (!bass.is_empty()).then(|| bass.to_string()),
            )
        }
        None => (s.trim(), None),
    };

    let (root, modifier) = match match_root(main_part) {
        Some(r) => {
            let rest = &main_part[r.len()..];
            let modifier = (!rest.is_empty()).then(|| rest.to_string());
            (r.to_string(), modifier)
        }
        None => {
            let root = main_part
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            let rest: String = main_part.chars().skip(1).collect();
            let modifier = (!rest.is_empty()).then_some(rest);
            (root, modifier)
        }
    };

    shorten_minor(Chord {
        root,
        modifier,
        bass,
    })
}

/// Czech songbooks name a minor chord "Emi" where the rest of the world names
/// it "Em", and every site read here writes it the Czech way. Nothing but the
/// spelling differs, so the short one is settled on as the chord is read and
/// no printer ever sees the long one.
///
/// A chord carries its minor in either of two places — behind the root, and
/// behind the slash in the bass note of "F#/Hmi".
pub(super) fn shorten_minor(chord: Chord) -> Chord {
    Chord {
        root: chord.root,
        modifier: chord
            .modifier
            .map(|modifier| match shortened_quality(&modifier) {
                Some(short) => short,
                None => modifier,
            }),
        bass: chord.bass.map(|bass| shorten_minor_name(&bass)),
    }
}

/// The same for a chord written out in full rather than read apart: a bass
/// note, or the key a page names the song's.
pub(super) fn shorten_minor_name(name: &str) -> String {
    let name = name.trim();
    match match_root(name).and_then(|root| {
        shortened_quality(&name[root.len()..]).map(|quality| format!("{}{}", root, quality))
    }) {
        Some(shortened) => shortened,
        None => name.to_string(),
    }
}

/// Shortens the "mi" of a chord's quality to "m", where that is what it is.
///
/// This reads what stands behind the root, never the chord itself: "mi" names
/// a minor chord only directly behind one, and a bare "mi" in a line of lyrics
/// is the Czech for "me".
fn shortened_quality(quality: &str) -> Option<String> {
    // "min" is the same word written a letter further.
    let rest = quality
        .strip_prefix("min")
        .or_else(|| quality.strip_prefix("mi"))?;

    // What follows has to belong to the chord — an added note, an accidental,
    // a bracket — or there has to be nothing at all. A letter would mean the
    // "mi" opened a word of its own, the "maj" of a minor-major chord aside.
    if rest.starts_with(|c: char| c.is_ascii_alphabetic()) && !rest.starts_with("maj") {
        return None;
    }

    Some(format!("m{}", rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Song {
        TextParser.parse("test", text)
    }

    fn chords(line: &Line) -> Vec<String> {
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
                name
            })
            .collect()
    }

    fn columns(line: &Line) -> Vec<usize> {
        line.chords.iter().map(|placed| placed.position).collect()
    }

    #[test]
    fn reads_backslash_bass_notes_and_expands_tabs() {
        let song = parse("Gm\tBb   C\\E\nWhen are you gonna come down\n");
        let line = &song.parts[0].lines[0];

        assert_eq!(line.lyrics, "When are you gonna come down");
        assert_eq!(chords(line), ["Gm", "Bb", "C/E"]);
        assert_eq!(columns(line), [0, 8, 13]);
    }

    #[test]
    fn reads_a_header_that_is_not_underlined() {
        let song = parse("GOODBYE YELLOW BRICK ROAD\nYear:  1973\nAlbum:  Same\n\nlyrics\n");

        assert_eq!(song.title, "GOODBYE YELLOW BRICK ROAD");
        assert_eq!(
            song.extra,
            [
                ("Year".to_string(), "1973".to_string()),
                ("Album".to_string(), "Same".to_string()),
            ]
        );
    }

    #[test]
    fn reads_an_underlined_artist_and_title() {
        let song = parse("Beatles - Let It Be\n===\nKey: C\nCapo: 2\n\nlyrics\n");

        assert_eq!(song.artist, "Beatles");
        assert_eq!(song.title, "Let It Be");
        assert_eq!(song.key.as_deref(), Some("C"));
        assert_eq!(song.capo, Some(2));
    }

    #[test]
    fn a_marker_may_share_its_line_with_chords() {
        let song = parse("R:  C      G\nLet it be\n");
        let part = &song.parts[0];

        assert!(matches!(part.part_type, PartType::Chorus));
        assert_eq!(part.lines[0].lyrics, "Let it be");
        assert_eq!(chords(&part.lines[0]), ["C", "G"]);
        assert_eq!(columns(&part.lines[0]), [4, 11]);
    }

    #[test]
    fn reads_the_meter_out_of_the_header() {
        let song = parse("Beatles - Let It Be\n===\nTime signature: 3 / 4\n\nlyrics\n");

        assert_eq!(song.meter.as_deref(), Some("3/4"));
    }

    #[test]
    fn a_meter_that_is_no_fraction_is_not_read() {
        assert_eq!(meter_of("waltz"), None);
        assert_eq!(meter_of("6/8 swing"), Some("6/8".to_string()));
    }

    #[test]
    fn a_verse_number_shifts_the_chords_above_it() {
        let song = parse("     C\n1. When I find myself\n");
        let line = &song.parts[0].lines[0];

        assert_eq!(line.lyrics, "When I find myself");
        assert_eq!(columns(line), [2]);
    }

    #[test]
    fn a_blank_line_starts_a_new_part() {
        let song = parse("one\n\ntwo\n");

        assert_eq!(song.parts.len(), 2);
    }

    #[test]
    fn a_bracket_belonging_to_the_chord_survives_the_line_end() {
        let song = parse("Em        B(7)\nthe long way home");

        assert_eq!(chords(&song.parts[0].lines[0]), ["Em", "B(7)"]);
    }

    #[test]
    fn an_aside_does_not_turn_a_chord_line_into_lyrics() {
        let song = parse("Em      D (not sure about this one)\nthe long way home");

        assert_eq!(chords(&song.parts[0].lines[0]), ["Em", "D"]);
        assert_eq!(columns(&song.parts[0].lines[0]), [0, 8]);
    }

    #[test]
    fn bar_lines_neither_hide_the_chords_nor_get_lost() {
        let song = parse("[Intro]\n| C  G | Am | x2\n");
        let line = &song.parts[0].lines[0];

        assert!(matches!(song.parts[0].part_type, PartType::Intro));
        assert_eq!(chords(line), ["C", "G", "Am"]);
        assert_eq!(line.lyrics.matches('|').count(), 3);
        assert!(line.lyrics.ends_with("x2"));
    }

    #[test]
    fn a_marker_may_be_written_in_brackets() {
        let song = parse("(1) When I find myself\n\n(R) Let it be\n\n(I) C  G\n");

        assert!(matches!(song.parts[0].part_type, PartType::Verse));
        assert_eq!(song.parts[0].lines[0].lyrics, "When I find myself");
        assert!(matches!(song.parts[1].part_type, PartType::Chorus));
        assert!(matches!(song.parts[2].part_type, PartType::Intro));
        assert_eq!(chords(&song.parts[2].lines[0]), ["C", "G"]);
    }

    #[test]
    fn a_block_with_nothing_to_sing_is_no_verse() {
        let song = parse("Dmi\n\nC  G\nLet it be\n\n| C  G | Am | x2\n");

        assert!(matches!(song.parts[0].part_type, PartType::Intro));
        assert!(matches!(song.parts[1].part_type, PartType::Verse));
        assert!(matches!(song.parts[2].part_type, PartType::Instrumental));
    }

    #[test]
    fn a_bracket_the_lyrics_open_is_no_marker() {
        let song = parse("(ona šla domů)\n");

        assert!(matches!(song.parts[0].part_type, PartType::Verse));
        assert_eq!(song.parts[0].lines[0].lyrics, "(ona šla domů)");
    }

    #[test]
    fn the_margin_a_part_shares_is_dropped_but_its_alignment_is_not() {
        let song = parse("      C           G\n1. When I find myself in times\n   of trouble\n");
        let lines = &song.parts[0].lines;

        assert_eq!(lines[0].lyrics, "When I find myself in times");
        assert_eq!(columns(&lines[0]), [3, 15]);
        // The second line was indented as far as the first one's marker.
        assert_eq!(lines[1].lyrics, "of trouble");
    }

    #[test]
    fn lyrics_starting_with_a_keyword_stay_lyrics() {
        let song = parse("Chorus of angels sing\n");

        assert!(matches!(song.parts[0].part_type, PartType::Verse));
        assert_eq!(song.parts[0].lines[0].lyrics, "Chorus of angels sing");
    }

    /// Writes a chord out the way the printers do, to read its pieces back.
    fn spelled(chord: &str) -> String {
        let chord = parse_chord(chord);
        let mut name = chord.root;
        name.push_str(chord.modifier.as_deref().unwrap_or(""));
        if let Some(bass) = chord.bass {
            name.push('/');
            name.push_str(&bass);
        }
        name
    }

    #[test]
    fn the_czech_minor_is_read_as_the_short_one() {
        assert_eq!(spelled("Emi"), "Em");
        assert_eq!(spelled("Ami7"), "Am7");
        assert_eq!(spelled("Hmi7"), "Hm7");
        assert_eq!(spelled("C#mi9"), "C#m9");
        assert_eq!(spelled("Cismi"), "Cism");
        // "min" is the same word written a letter further.
        assert_eq!(spelled("Dmin"), "Dm");
        assert_eq!(spelled("Dmin7"), "Dm7");
        // A minor-major keeps its "maj" and gives up only the "mi".
        assert_eq!(spelled("Cmimaj7"), "Cmmaj7");
    }

    #[test]
    fn a_bass_note_gives_its_minor_up_too() {
        assert_eq!(spelled("Hmi7/Fis"), "Hm7/Fis");
        assert_eq!(spelled("F#/Hmi"), "F#/Hm");
    }

    #[test]
    fn a_chord_that_never_was_minor_is_left_as_it_was() {
        for chord in [
            "Em", "Am7", "Cmaj7", "C", "Fis", "Bb", "Dsus4", "Gadd9", "D/F#",
        ] {
            assert_eq!(spelled(chord), chord, "{} was not left alone", chord);
        }
    }

    /// "N.C." — *no chord* — is written where a chord would stand and is read
    /// as one, so that the line it is on still reads as a line of chords and
    /// it still keeps the column it was written in.
    #[test]
    fn the_no_chord_mark_is_read_as_a_chord() {
        let song = parse("Em      N.C.      G\nthe wind knows my name\n");
        let line = &song.parts[0].lines[0];

        assert_eq!(line.lyrics, "the wind knows my name");
        assert_eq!(chords(line), ["Em", "N.C.", "G"]);
        assert_eq!(columns(line), [0, 8, 18]);
    }

    /// A bar played on nothing at all is a line of its own, and there is no
    /// chord on it to make it one.
    #[test]
    fn a_line_holding_nothing_but_the_mark_is_still_a_chord_line() {
        let song = parse("[Bridge]\n| N.C. |\n\nlyrics\n");

        let line = &song.parts[0].lines[0];
        assert_eq!(chords(line), ["N.C."]);
        // The bars stay; the mark is taken out from between them like a chord.
        assert_eq!(line.lyrics.trim_end(), "|      |");
    }

    /// The mark is written with as many of its dots as whoever wrote the sheet
    /// felt like — and all of them mean the one thing.
    #[test]
    fn the_mark_is_read_however_it_is_spelled() {
        for spelling in ["N.C.", "N.C", "NC", "N/C", "n.c."] {
            let chord = parse_chord(spelling);
            assert!(chord.is_no_chord(), "{} was not read as no chord", spelling);
            assert_eq!(spelled(spelling), "N.C.");
        }
    }

    /// A word of the lyrics is not the mark, however it starts.
    #[test]
    fn a_word_that_merely_opens_with_an_n_is_left_to_the_lyrics() {
        let song = parse("       G\nNo chord and no name\n");

        assert_eq!(song.parts[0].lines[0].lyrics, "No chord and no name");
        assert_eq!(chords(&song.parts[0].lines[0]), ["G"]);
    }

    /// The shortening reads what stands behind a root, never a line of its
    /// own: "mi" is the Czech for "me" and turns up in the lyrics constantly.
    #[test]
    fn a_mi_in_the_lyrics_is_no_chord() {
        let song = parse("       Ami\nona mi zmizela\n");

        assert_eq!(song.parts[0].lines[0].lyrics, "ona mi zmizela");
        assert_eq!(spelled("Ami"), "Am");
    }
}
