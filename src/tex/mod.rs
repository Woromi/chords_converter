//! Reading a song the songbook already holds, `.tex` and all, so it can be
//! improved where it stands. Nothing here prints a song again: a song may
//! have been edited by hand, and every byte nobody meant to change has to
//! stay as it was. So a song is *scanned*, every part found together with
//! the bytes it spans, and a change is a replacement of exactly those bytes.

pub mod fit;
pub mod measure;
pub mod tidy;

use std::ops::Range;

/// One part of a song, `\beginchorus` … `\endchorus` and the like.
#[derive(Debug, Clone, PartialEq)]
pub struct PartSpan {
    /// The part's name as the macro spells it: `chorus`, `verse*`.
    pub kind: String,
    /// The `[n]` of a numbered chorus.
    pub number: Option<u32>,
    /// From the backslash of `\begin…` to the end of `\end…`.
    pub span: Range<usize>,
    /// What follows the `\begin…[n]` on its line (`\chordsoffonsmallpage`),
    /// comment included.
    pub opening_rest: Range<usize>,
    /// The lines between the opening and the closing line.
    pub body: Range<usize>,
    /// What follows the `\end…` on its line.
    pub closing_rest: Range<usize>,
    /// Whether the part sits inside an `\if… \fi`: what it typesets depends
    /// on more than the song, so nothing may lean on it.
    pub conditional: bool,
    /// The 1-based line the part opens on, for reporting.
    pub line: usize,
}

/// The value of a `lang` key on a prelude line: `lang={czech}`, as the
/// songbook writes it, or `lang=czech` and `lang = czech`, which the
/// prelude's key=value options take as well.
fn lang_of(code: &str) -> Option<String> {
    for (at, _) in code.match_indices("lang") {
        // The key itself, not the end of another word.
        if code[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '\\')
        {
            continue;
        }
        let Some(rest) = code[at + 4..].trim_start().strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.strip_prefix('{') {
            Some(braced) => &braced[..braced.find('}')?],
            None => rest.split([',', ']']).next().unwrap_or(""),
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// A song as scanned.
#[derive(Debug, Default)]
pub struct Scan {
    pub parts: Vec<PartSpan>,
    /// Every `\insertchorus…` outside a comment, and whether it is
    /// conditional.
    pub chorus_repeats: Vec<(Range<usize>, bool)>,
    /// The `lang` of the prelude, if set.
    pub lang: Option<String>,
}

/// The bytes of a line that are not comment: everything before the first `%`
/// no backslash escapes. (`\%` and `\[\%]` are a percent sign, not a comment.)
pub fn code_of(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'%' {
            let backslashes = bytes[..i].iter().rev().take_while(|&&c| c == b'\\').count();
            if backslashes % 2 == 0 {
                return &line[..i];
            }
        }
    }
    line
}

/// The control words in a stretch of code, each with where it starts:
/// `\beginchorus` gives `beginchorus`.
fn control_words(code: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                end += 1;
            }
            if end > start {
                words.push((i, &code[start..end]));
                i = end;
                continue;
            }
            // A control symbol (`\\`, `\%`, `\[`): skip the symbol too.
            i = start + 1;
            continue;
        }
        i += 1;
    }
    words
}

/// Whether a control word opens a conditional. `\ifx`, `\iflargepaper`,
/// `\ifSB@inchorus` … all start with "if"; a word that merely does
/// (`\iffalse` aside) is not one the songs use.
fn opens_conditional(word: &str) -> bool {
    word.len() > 2 && word.starts_with("if")
}

/// Scans a song. A song it can't make sense of (a part never closed) gives
/// the parts it could; the tidy rules only touch parts that were whole.
pub fn scan(text: &str) -> Scan {
    let mut scan = Scan::default();
    let mut depth: usize = 0;
    // The part being read, filled in as far as its opening line tells.
    let mut open: Option<PartSpan> = None;

    let mut offset = 0;
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        let line_start = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let code = code_of(line);

        if scan.lang.is_none() {
            scan.lang = lang_of(code);
        }

        let words = control_words(code);
        for &(at, word) in &words {
            let absolute = line_start + at;
            if opens_conditional(word) {
                depth += 1;
            } else if word == "fi" {
                depth = depth.saturating_sub(1);
            } else if word == "insertchorus" {
                let mut end = at + 1 + word.len();
                let rest = &code[end..];
                if rest.starts_with('*') {
                    end += 1;
                }
                // `[n]`, then `[setup]` run inside the reprint.
                for _ in 0..2 {
                    if code[end..].starts_with('[')
                        && let Some(close) = code[end..].find(']')
                    {
                        end += close + 1;
                    } else {
                        break;
                    }
                }
                scan.chorus_repeats
                    .push((absolute..line_start + end, depth > 0));
            } else if let Some(kind) = word.strip_prefix("begin") {
                if kind.is_empty() || kind == "song" || open.is_some() {
                    continue;
                }
                let mut after = at + 1 + word.len();
                let mut kind = kind.to_string();
                if code[after..].starts_with('*') {
                    kind.push('*');
                    after += 1;
                }
                let mut number = None;
                if code[after..].starts_with('[')
                    && let Some(close) = code[after..].find(']')
                {
                    number = code[after + 1..after + close].trim().parse().ok();
                    if number.is_none() {
                        // An option that isn't a number: not a part we know.
                        continue;
                    }
                    after += close + 1;
                }
                open = Some(PartSpan {
                    kind,
                    number,
                    span: absolute..absolute,
                    opening_rest: line_start + after..line_start + line.len(),
                    body: offset..offset,
                    closing_rest: offset..offset,
                    conditional: depth > 0,
                    line: index + 1,
                });
            } else if let Some(kind) = word.strip_prefix("end")
                && let Some(part) = &open
                && part.kind.trim_end_matches('*') == kind
            {
                let mut part = open.take().unwrap();
                let mut end = at + 1 + word.len();
                if code[end..].starts_with('*') {
                    end += 1;
                }
                part.span.end = line_start + end;
                part.body = part.body.start.min(line_start)..line_start;
                part.closing_rest = line_start + end..line_start + line.len();
                part.conditional |= depth > 0;
                scan.parts.push(part);
            }
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = "\\beginsong{T}[\n    lang={czech},\n]\n\n\\beginverse\n    \\[C]Jedna 50\\% \\[\\%]\n\\endverse\n\n\\beginchorus[2]\\chordsoffonsmallpage % note\n    \\[G]Ref\n\\endchorus \\rep{2}\n\n\\iflargepaper\n\\beginchorus\n    x\n\\endchorus\n\\fi\n\\insertchorus*[2]\n% \\beginverse\n\\endsong\n";

    #[test]
    fn parts_are_found_with_their_bytes() {
        let scan = scan(SONG);
        assert_eq!(scan.lang.as_deref(), Some("czech"));
        // With or without braces, with or without spaces, on one line with
        // the other keys: the prelude takes them all.
        for header in [
            "\\beginsong{K}[by={Test}, key=C, lang=czech]",
            "\\beginsong{K}[by={Test}, key=C, lang={czech}]",
            "    lang = czech,",
            "    lang = { czech },",
        ] {
            assert_eq!(lang_of(header).as_deref(), Some("czech"), "{header}");
        }
        assert_eq!(lang_of("    slang=x,"), None);
        let kinds: Vec<_> = scan.parts.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["verse", "chorus", "chorus"]);

        let chorus = &scan.parts[1];
        assert_eq!(chorus.number, Some(2));
        assert_eq!(chorus.line, 9);
        assert_eq!(
            &SONG[chorus.span.clone()],
            "\\beginchorus[2]\\chordsoffonsmallpage % note\n    \\[G]Ref\n\\endchorus"
        );
        assert_eq!(
            &SONG[chorus.opening_rest.clone()],
            "\\chordsoffonsmallpage % note"
        );
        assert_eq!(&SONG[chorus.body.clone()], "    \\[G]Ref\n");
        assert_eq!(&SONG[chorus.closing_rest.clone()], " \\rep{2}");
        assert!(!chorus.conditional);
        assert!(scan.parts[2].conditional);
    }

    #[test]
    fn an_escaped_percent_is_no_comment() {
        assert_eq!(code_of("50\\% \\[\\%] x % y"), "50\\% \\[\\%] x ");
        assert_eq!(code_of("a\\\\% b"), "a\\\\");
        let scan = scan(SONG);
        assert_eq!(
            &SONG[scan.parts[0].body.clone()],
            "    \\[C]Jedna 50\\% \\[\\%]\n"
        );
    }

    #[test]
    fn chorus_repeats_and_commented_parts() {
        let scan = scan(SONG);
        assert_eq!(scan.chorus_repeats.len(), 1);
        assert_eq!(
            &SONG[scan.chorus_repeats[0].0.clone()],
            "\\insertchorus*[2]"
        );
        // The commented-out \beginverse is no part.
        assert_eq!(scan.parts.len(), 3);
    }

    #[test]
    fn a_repeat_with_setup_spans_both_brackets() {
        let text = "\\insertchorus[][\\chordsoff] \\rep{2}\n";
        let scan = scan(text);
        assert_eq!(
            &text[scan.chorus_repeats[0].0.clone()],
            "\\insertchorus[][\\chordsoff]"
        );
    }

    /// Every song in the songbook next door scans, and each part re-slices to
    /// exactly its `\begin…` … `\end…`. Run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "reads ../ZubuvZpevnik"]
    fn every_songbook_song_scans() {
        fn songs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    songs(&path, out);
                } else if path.extension().is_some_and(|e| e == "tex") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        songs(
            std::path::Path::new("../ZubuvZpevnik/songs/chords"),
            &mut files,
        );
        assert!(files.len() > 100, "songbook not found");

        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let scan = scan(&text);
            let begins = text
                .lines()
                .map(code_of)
                .flat_map(control_words)
                .filter(|(_, w)| w.starts_with("begin") && *w != "beginsong" && *w != "begin")
                .count();
            assert_eq!(scan.parts.len(), begins, "{}", file.display());
            for part in &scan.parts {
                let whole = &text[part.span.clone()];
                assert!(whole.starts_with("\\begin"), "{}: {whole}", file.display());
                assert!(
                    whole
                        .trim_end_matches('*')
                        .ends_with(part.kind.trim_end_matches('*')),
                    "{}: {whole}",
                    file.display()
                );
                assert!(part.body.start <= part.body.end);
            }
        }
    }
}
