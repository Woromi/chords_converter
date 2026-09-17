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
    let mut current_lines: Vec<Line> = Vec::new();

    while idx < lines.len() {
        let line = lines[idx].clone();
        let line = line.as_str();
        let trimmed = line.trim();

        // A. Blank line: closes the current block, keeps the section type
        //    so that a multi-stanza chorus stays a chorus.
        if trimmed.is_empty() {
            flush_part(&mut parts, &current_type, &mut current_lines);
            idx += 1;
            continue;
        }

        // B. Bracketed section marker, e.g. "[Chorus]" or "[Verse 2]".
        if let Some(pt) = parse_bracket_marker(trimmed) {
            flush_part(&mut parts, &current_type, &mut current_lines);
            current_type = pt;
            idx += 1;
            continue;
        }

        // C. Inline structural marker, e.g. "R:", "1." (Czech/English).
        if let Some((pt, rest_at)) = detect_part_type(line) {
            flush_part(&mut parts, &current_type, &mut current_lines);
            current_type = pt;
            let rest = &line[rest_at..];
            if is_chord_line(rest) {
                // "Intro: | C G | Am F |" or "R:  C      G" — blank the
                // marker out and read the line again, so the chords keep
                // their columns and can still pair with the lyrics below.
                let marker_width = line[..rest_at].chars().count();
                lines[idx] = " ".repeat(marker_width) + rest;
                continue;
            }
            if !rest.trim().is_empty() {
                current_lines.push(Line {
                    lyrics: rest.trim().to_string(),
                    chords: Vec::new(),
                });
            }
            idx += 1;
            continue;
        }

        // D. Tablature: a run of string lines, taken verbatim into a part of
        //    its own. Pairing it with the chords above it as though it were
        //    lyrics would write them into the middle of the staff.
        if is_tablature_line(line) {
            flush_part(&mut parts, &current_type, &mut current_lines);

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
                lines: staff,
            });
            continue;
        }

        // E. Chord line: attach it to the lyric line beneath, or keep it as
        //    an instrumental (chords-only) line when no lyrics follow.
        if is_chord_line(line) {
            let chords = parse_chord_line(line);
            let next = lines.get(idx + 1).map(String::as_str).unwrap_or("");
            let next_trim = next.trim();

            let follows_lyrics = !next_trim.is_empty()
                && !is_chord_line(next)
                && !is_tablature_line(next)
                && parse_bracket_marker(next_trim).is_none();

            if !follows_lyrics {
                current_lines.push(Line {
                    lyrics: decoration_skeleton(line),
                    chords,
                });
                idx += 1;
                continue;
            }

            // The lyric line below may carry a marker, as in "1. When I
            // find myself ...". Drop the marker and shift the chords left
            // by as many columns, keeping them above the right syllables.
            let (lyrics, chords) = match detect_part_type(next) {
                Some((pt, rest_at)) => {
                    let rest = next[rest_at..].trim_start();
                    if rest.is_empty() {
                        current_lines.push(Line {
                            lyrics: String::new(),
                            chords,
                        });
                        idx += 1;
                        continue;
                    }
                    flush_part(&mut parts, &current_type, &mut current_lines);
                    current_type = pt;
                    let dropped = next[..next.len() - rest.len()].chars().count();
                    (rest.trim_end().to_string(), shift_chords(chords, dropped))
                }
                None => (next.trim_end().to_string(), chords),
            };

            current_lines.push(Line { lyrics, chords });
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

    flush_part(&mut parts, &current_type, &mut current_lines);

    parts
}

/// Metadata gathered from the header, before the body is parsed.
struct Header {
    title: String,
    artist: String,
    source_website: String,
    key: Option<String>,
    tempo: Option<String>,
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
            "key" | "tónina" | "tonina" => self.key = Some(value),
            "tempo" | "bpm" => self.tempo = Some(value),
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
            language: self.language,
            tuning: self.tuning,
            capo: self.capo,
            extra: self.extra,
            parts,
        }
    }
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

/// Pushes the accumulated lines as a finished part (no-op when empty).
fn flush_part(parts: &mut Vec<Part>, current_type: &PartType, current_lines: &mut Vec<Line>) {
    if !current_lines.is_empty() {
        parts.push(Part {
            part_type: current_type.clone(),
            lines: std::mem::take(current_lines),
        });
    }
}

/// Moves chords `columns` to the left, used when a marker prefix is stripped
/// off the lyric line underneath them.
fn shift_chords(chords: Vec<PlacedChord>, columns: usize) -> Vec<PlacedChord> {
    chords
        .into_iter()
        .map(|placed| PlacedChord {
            position: placed.position.saturating_sub(columns),
            ..placed
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
    let inner = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
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
/// `.`, `)` or the end of the line. Returns the part type together with the
/// byte offset at which the rest of the line (if any) continues, so that
/// "1. Some lyrics" keeps its lyrics.
fn detect_part_type(line: &str) -> Option<(PartType, usize)> {
    let indent = line.len() - line.trim_start().len();
    let body = &line[indent..];

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
    match separator {
        Some(':') | Some('.') | Some(')') => cursor += 1,
        // A marker may also stand alone on its line, e.g. "Intro".
        None => {}
        _ => return None,
    }

    let marker = if keyword.is_empty() {
        // A bare number is only a marker when it is numbered and separated,
        // "1." starts a verse but "1973" is just text.
        if digits == 0 || separator.is_none() {
            return None;
        }
        PartMarker::Verse
    } else {
        lookup_marker(&keyword)?
    };

    Some((marker.part_type(), indent + cursor))
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
            for c in chars.iter_mut().skip(start).take(chord.chars().count()) {
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
fn parse_chord(s: &str) -> Chord {
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

    Chord {
        root,
        modifier,
        bass,
    }
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
    fn lyrics_starting_with_a_keyword_stay_lyrics() {
        let song = parse("Chorus of angels sing\n");

        assert!(matches!(song.parts[0].part_type, PartType::Verse));
        assert_eq!(song.parts[0].lines[0].lyrics, "Chorus of angels sing");
    }
}
