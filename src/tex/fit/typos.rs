//! Typos, for `check`: what the song itself says may be a slip, and what
//! breaks the songbook's own typography. A person decides on each, so they
//! are proposals like the others, and nothing here looks words up in a
//! dictionary: the song is checked against itself.
//!
//! - A line sung more than once (a chorus typed out again, a verse's line
//!   repeated) that differs from its other copies in one word, by an
//!   apostrophe, diacritics or one letter: `Ill` beside `I'll`. The rarer
//!   form is offered the commoner one; with as many of each, the fuller
//!   (with the apostrophe or the diacritics, or the longer word).
//! - Apostrophes typed both `'` and `’`: all made the commoner kind. TeX
//!   prints both alike, so this is about the source only.
//! - `~` after a word in an English song: ties are for Czech and Slovak
//!   (CLAUDE.md), never English. `\lrep~` and `~` used for spacing stay.

use super::{Edit, Names, PartInfo, Proposal, Stage, code_of};
use std::collections::HashMap;
use std::ops::Range;

/// A lyric word: its letters (chords and macros left out), and its bytes.
#[derive(Debug, Clone)]
struct Word {
    text: String,
    span: Range<usize>,
    /// No chord inside it, so its bytes can be replaced as they are.
    whole: bool,
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '’'
}

/// The words of a line's code, at `start` in the song.
fn words_of(code: &str, start: usize) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current: Option<Word> = None;
    let chars: Vec<(usize, char)> = code.char_indices().collect();
    let mut k = 0;
    let finish = |current: &mut Option<Word>, words: &mut Vec<Word>| {
        if let Some(mut word) = current.take() {
            // An apostrophe at the end is a closing quote, not the word's.
            while word.text.ends_with(is_apostrophe) {
                let c = word.text.pop().unwrap();
                word.span.end -= c.len_utf8();
            }
            if !word.text.is_empty() {
                words.push(word);
            }
        }
    };
    while k < chars.len() {
        let (at, c) = chars[k];
        if c == '\\' {
            let next = chars.get(k + 1).map(|&(_, n)| n);
            if next == Some('[') {
                // A chord: inside a word, the word goes on after it.
                let mut j = k + 2;
                while j < chars.len() && chars[j].1 != ']' {
                    j += 1;
                }
                k = j + 1;
                let letter_after = chars.get(k).is_some_and(|&(_, n)| n.is_alphabetic());
                match &mut current {
                    Some(word) if letter_after => word.whole = false,
                    _ => finish(&mut current, &mut words),
                }
                continue;
            }
            finish(&mut current, &mut words);
            if next.is_some_and(|n| n.is_ascii_alphabetic()) {
                k += 1;
                while chars.get(k).is_some_and(|&(_, n)| n.is_ascii_alphabetic()) {
                    k += 1;
                }
            } else {
                k += 2;
            }
            continue;
        }
        let in_word = c.is_alphabetic()
            || (is_apostrophe(c)
                && current.is_some()
                && chars.get(k + 1).is_some_and(|&(_, n)| n.is_alphabetic()));
        if in_word {
            let word = current.get_or_insert_with(|| Word {
                text: String::new(),
                span: start + at..start + at,
                whole: true,
            });
            word.text.push(c);
            word.span.end = start + at + c.len_utf8();
        } else {
            finish(&mut current, &mut words);
        }
        k += 1;
    }
    finish(&mut current, &mut words);
    words
}

/// A letter without its diacritics, for Czech, Slovak and the like.
fn plain(c: char) -> char {
    match c {
        'á' | 'ä' | 'à' | 'â' => 'a',
        'č' | 'ç' => 'c',
        'ď' => 'd',
        'é' | 'ě' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ľ' | 'ĺ' => 'l',
        'ň' | 'ñ' => 'n',
        'ó' | 'ô' | 'ö' | 'ò' => 'o',
        'ř' | 'ŕ' => 'r',
        'š' => 's',
        'ť' => 't',
        'ú' | 'ů' | 'ü' | 'ù' | 'û' => 'u',
        'ý' | 'ÿ' => 'y',
        'ž' => 'z',
        c => c,
    }
}

fn without_apostrophes(w: &str) -> String {
    w.chars().filter(|&c| !is_apostrophe(c)).collect()
}

fn without_diacritics(w: &str) -> String {
    w.chars().map(plain).collect()
}

/// Edit distance in letters.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut next = vec![i + 1];
        for (j, y) in b.iter().enumerate() {
            next.push(
                (row[j] + usize::from(x != *y))
                    .min(row[j + 1] + 1)
                    .min(next[j] + 1),
            );
        }
        row = next;
    }
    row[b.len()]
}

/// Whether two words (lowercased) look like one of them mistyped.
fn looks_like_a_slip(a: &str, b: &str) -> bool {
    a != b
        && (without_apostrophes(a) == without_apostrophes(b)
            || without_diacritics(a) == without_diacritics(b)
            || (a.chars().count().min(b.chars().count()) >= 4 && distance(a, b) == 1))
}

/// Of two forms as common, the one more likely right: with the apostrophe,
/// with the diacritics, or the longer. `None` when nothing tells.
fn fuller<'w>(a: &'w str, b: &'w str) -> Option<&'w str> {
    let count = |w: &str, f: fn(char) -> bool| w.chars().filter(|&c| f(c)).count();
    let marks = |c: char| plain(c) != c;
    let pick = |x: usize, y: usize| match x.cmp(&y) {
        std::cmp::Ordering::Greater => Some(a),
        std::cmp::Ordering::Less => Some(b),
        std::cmp::Ordering::Equal => None,
    };
    pick(count(a, is_apostrophe), count(b, is_apostrophe))
        .or_else(|| pick(count(a, marks), count(b, marks)))
        .or_else(|| pick(a.chars().count(), b.chars().count()))
}

/// `with`'s letters, in the case `like` starts with.
fn in_case_of(with: &str, like: &str) -> String {
    let upper = like.chars().next().is_some_and(char::is_uppercase);
    let mut chars = with.chars();
    match chars.next() {
        Some(first) if upper => first.to_uppercase().chain(chars).collect(),
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A sung line: its number and its words.
struct Sung {
    line: usize,
    part: usize,
    words: Vec<Word>,
}

fn sung_lines(text: &str, infos: &[PartInfo]) -> Vec<Sung> {
    let mut lines = Vec::new();
    for (i, info) in infos.iter().enumerate() {
        if info.part.kind.starts_with("instrumental") {
            continue;
        }
        let body = info.part.body.clone();
        let mut at = body.start;
        for raw in text[body.clone()].split_inclusive('\n') {
            let code = code_of(raw.trim_end_matches(['\n', '\r']));
            let line = text[..at].matches('\n').count() + 1;
            let words = words_of(code, at);
            at += raw.len();
            if !words.is_empty() {
                lines.push(Sung {
                    line,
                    part: i,
                    words,
                });
            }
        }
    }
    lines
}

fn correction(
    id: String,
    kind: &'static str,
    line: usize,
    description: String,
    edits: Vec<Edit>,
) -> Proposal {
    Proposal {
        id,
        kind,
        stage: Stage::AllPapers,
        line,
        description,
        gain_lines: 0,
        score: 0,
        edits,
        sources: Vec::new(),
        effects: Vec::new(),
        joins: None,
    }
}

pub(super) fn typos(infos: &[PartInfo], names: &Names, lang: Option<&str>) -> Vec<Proposal> {
    let text = names.text;
    let lines = sung_lines(text, infos);
    let mut proposals = copies_differing(&lines, names);
    proposals.extend(mixed_apostrophes(&lines, text));
    if lang.is_some_and(|l| l.eq_ignore_ascii_case("english")) {
        proposals.extend(ties_in_english(text, infos));
    }
    if lang.is_some_and(czech_like) {
        proposals.extend(missing_commas(&lines, text, names));
    }
    proposals
}

fn czech_like(lang: &str) -> bool {
    matches!(lang.to_lowercase().as_str(), "czech" | "slovak")
}

/// Whether a comma goes before `word` when it starts a clause, by the
/// language's rule: `Some(true)` before a word opening a subordinate clause
/// in Czech or Slovak (že, když, který…) or before "ale"/"but",
/// `Some(false)` before the conjunctions that join without one (a, i,
/// nebo / and, or), `None` where no rule tells (a person decides).
pub(super) fn comma_before(lang: Option<&str>, word: &str) -> Option<bool> {
    let word = word.to_lowercase();
    let w = word.as_str();
    match lang {
        Some(l) if czech_like(l) => {
            if CZECH_COMMA.contains(&w) || w.starts_with("kter") || w.starts_with("ktor") {
                Some(true)
            } else if ["a", "i", "nebo", "ani", "či", "alebo"].contains(&w) {
                Some(false)
            } else {
                None
            }
        }
        Some(l) if l.eq_ignore_ascii_case("english") => match w {
            "but" => Some(true),
            "and" | "or" | "nor" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Czech and Slovak words that open a clause with a comma before it.
const CZECH_COMMA: &[&str] = &[
    "že",
    "protože",
    "aby",
    "abych",
    "abys",
    "abychom",
    "abyste",
    "kdyby",
    "kdybych",
    "kdybys",
    "kdybychom",
    "kdybyste",
    "když",
    "jenž",
    "jež",
    "jehož",
    "jejž",
    "ale",
    "avšak",
    "ať",
    "jestli",
    "pokud",
    "dokud",
    "zatímco",
    "takže",
    "jakmile",
    "ačkoli",
    "ačkoliv",
    "přestože",
    "kdežto",
    "neboť",
    "keď",
    "lebo",
    "pretože",
    "hoci",
];

/// Words after which a clause word takes no comma between the two: a
/// preposition, whose comma goes before it ("místo, na kterém"), and the
/// particles and adverbs that lead into the clause ("tak ať", "jen aby",
/// "pak když").
const NO_COMMA_AFTER: &[&str] = &[
    "na",
    "ze",
    "z",
    "v",
    "ve",
    "s",
    "se",
    "k",
    "ke",
    "o",
    "u",
    "do",
    "od",
    "po",
    "pro",
    "při",
    "za",
    "před",
    "pod",
    "nad",
    "bez",
    "přes",
    "mezi",
    "kvůli",
    "podle",
    "proti",
    "skrz",
    "mimo",
    "tak",
    "pak",
    "jen",
    "jenom",
    "už",
    "ještě",
    "právě",
    "hlavně",
    "zvlášť",
    "teprve",
    "až",
    "asi",
    "snad",
    "prostě",
    "kolik",
    "proto",
    "zase",
    "potom",
    "teď",
    "tam",
    "tu",
    "proč",
    "jak",
    "však",
    "tedy",
    "jako",
    "nebo",
];

/// Inside a Czech or Slovak line, a word opening a clause with no comma
/// before it: "vím že" for "vím, že". Not after a, i, nebo (`i když`), nor
/// at the start of a line, whose end stands for the comma.
fn missing_commas(lines: &[Sung], text: &str, names: &Names) -> Vec<Proposal> {
    let mut proposals = Vec::new();
    for sung in lines {
        for pair in sung.words.windows(2) {
            let (before, word) = (&pair[0], &pair[1]);
            let w = word.text.to_lowercase();
            // "ale" inside a clause is a particle ("nejdřív ale musím").
            let opens = (CZECH_COMMA.contains(&w.as_str())
                && !["ale", "avšak"].contains(&w.as_str()))
                || w.starts_with("kter")
                || w.starts_with("ktor");
            let prev = before.text.to_lowercase();
            if !opens
                || comma_before(Some("czech"), &prev) == Some(false)
                || NO_COMMA_AFTER.contains(&prev.as_str())
            {
                continue;
            }
            // Only a space, chords or a tie between the two words: no
            // punctuation is there already.
            let between = &text[before.span.end..word.span.start];
            let bare = between
                .split("\\[")
                .map(|s| s.split_once(']').map_or(s, |(_, rest)| rest))
                .all(|s| s.chars().all(|c| c.is_whitespace() || c == '~'));
            if !bare || !before.whole {
                continue;
            }
            proposals.push(correction(
                format!("comma@{}", sung.line),
                "comma",
                sung.line,
                format!(
                    "{}: \"{} {}\" may want a comma before \"{}\"",
                    names.parts[sung.part], before.text, word.text, word.text
                ),
                vec![Edit {
                    span: before.span.end..before.span.end,
                    replacement: ",".to_string(),
                }],
            ));
        }
    }
    proposals
}

/// Lines sung more than once whose copies differ in one word, by a slip.
fn copies_differing(lines: &[Sung], names: &Names) -> Vec<Proposal> {
    let key = |s: &Sung| -> Vec<String> { s.words.iter().map(|w| w.text.to_lowercase()).collect() };
    // Each form of a line, and where it is sung.
    let mut forms: Vec<(Vec<String>, Vec<usize>)> = Vec::new();
    for (n, sung) in lines.iter().enumerate() {
        if sung.words.len() < 3 {
            continue;
        }
        let k = key(sung);
        match forms.iter_mut().find(|(f, _)| *f == k) {
            Some((_, at)) => at.push(n),
            None => forms.push((k, vec![n])),
        }
    }

    // Each slip found: the line it is on, the line it differs from, and
    // which word.
    let mut slips: Vec<(usize, usize, usize)> = Vec::new();
    for a in 0..forms.len() {
        for b in a + 1..forms.len() {
            let (fa, la) = &forms[a];
            let (fb, lb) = &forms[b];
            if fa.len() != fb.len() {
                continue;
            }
            let differing: Vec<usize> = (0..fa.len()).filter(|&i| fa[i] != fb[i]).collect();
            let [i] = differing[..] else { continue };
            if !looks_like_a_slip(&fa[i], &fb[i]) {
                continue;
            }
            // Which is the slip: the rarer, or by the form of the word.
            let (wrong, right) = match la.len().cmp(&lb.len()) {
                std::cmp::Ordering::Less => (la, lb),
                std::cmp::Ordering::Greater => (lb, la),
                std::cmp::Ordering::Equal => match fuller(&fa[i], &fb[i]) {
                    Some(w) if w == fa[i] => (lb, la),
                    Some(_) => (la, lb),
                    None => continue,
                },
            };
            for &n in wrong {
                slips.push((n, right[0], i));
            }
        }
    }
    // Two parts that differ in more than one line differ on purpose (a
    // chorus sung to "you" the second time: "toulá" / "touláš"), not by a
    // slip of the finger.
    let parts = |n: usize, m: usize| (lines[n].part, lines[m].part);
    let found = slips.clone();
    slips.retain(|&(n, m, _)| {
        found
            .iter()
            .filter(|&&(n2, m2, _)| parts(n2, m2) == parts(n, m) || parts(m2, n2) == parts(n, m))
            .count()
            == 1
    });

    let mut proposals = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    for (n, m, i) in slips {
        let model = &lines[m];
        let right_word = &model.words[i].text;
        let sung = &lines[n];
        let word = &sung.words[i];
        let fixed = in_case_of(right_word, &word.text);
        let (edits, how) = if word.whole {
            (
                vec![Edit {
                    span: word.span.clone(),
                    replacement: fixed.clone(),
                }],
                format!("make it \"{fixed}\""),
            )
        } else {
            (
                Vec::new(),
                "a chord sits inside the word, so fix it by hand".to_string(),
            )
        };
        let base = format!("typo@{}", sung.line);
        let seen = ids.entry(base.clone()).or_insert(0);
        *seen += 1;
        let id = if *seen == 1 {
            base
        } else {
            format!("{base}.{seen}")
        };
        proposals.push(correction(
            id,
            "typo",
            sung.line,
            format!(
                "{} sings \"{}\" where {} sings \"{}\" (line {}): {how}",
                names.parts[sung.part], word.text, names.parts[model.part], right_word, model.line
            ),
            edits,
        ));
    }
    proposals
}

/// Apostrophes typed both ways: all made the commoner kind.
fn mixed_apostrophes(lines: &[Sung], text: &str) -> Vec<Proposal> {
    let mut by_kind: [Vec<(usize, Range<usize>)>; 2] = [Vec::new(), Vec::new()];
    for sung in lines {
        for word in &sung.words {
            if !word.whole {
                continue;
            }
            for (at, c) in text[word.span.clone()].char_indices() {
                if is_apostrophe(c) {
                    let start = word.span.start + at;
                    by_kind[usize::from(c == '’')].push((sung.line, start..start + c.len_utf8()));
                }
            }
        }
    }
    let [straight, curly] = &by_kind;
    if straight.is_empty() || curly.is_empty() {
        return Vec::new();
    }
    // As many of each: straight, the songbook's commoner.
    let (to, from, from_char) = if curly.len() > straight.len() {
        ("’", straight, '\'')
    } else {
        ("'", curly, '’')
    };
    let edits = from
        .iter()
        .map(|(_, span)| Edit {
            span: span.clone(),
            replacement: to.to_string(),
        })
        .collect();
    vec![correction(
        format!("apostrophes@{}", from[0].0),
        "apostrophes",
        from[0].0,
        format!(
            "the song types apostrophes both ways, {} {from_char} and {} {to}: make them all {to}",
            from.len(),
            if to == "’" {
                curly.len()
            } else {
                straight.len()
            },
        ),
        edits,
    )]
}

/// `~` after a word in an English song: a space.
fn ties_in_english(text: &str, infos: &[PartInfo]) -> Vec<Proposal> {
    let mut edits = Vec::new();
    let mut first_line = None;
    for info in infos {
        let body = info.part.body.clone();
        let mut at = body.start;
        for raw in text[body.clone()].split_inclusive('\n') {
            let code = code_of(raw);
            let bytes = code.as_bytes();
            for (k, _) in code.match_indices('~') {
                // After letters that are a word, not a control word's name.
                let letters = code[..k]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphabetic())
                    .map(char::len_utf8)
                    .sum::<usize>();
                let word_before = letters > 0 && (k == letters || bytes[k - letters - 1] != b'\\');
                let after = code[k + 1..].chars().next();
                let word_after =
                    after.is_some_and(|c| c.is_alphabetic()) || code[k + 1..].starts_with("\\[");
                if word_before && word_after {
                    edits.push(Edit {
                        span: at + k..at + k + 1,
                        replacement: " ".to_string(),
                    });
                    first_line.get_or_insert(text[..at].matches('\n').count() + 1);
                }
            }
            at += raw.len();
        }
    }
    let Some(line) = first_line else {
        return Vec::new();
    };
    let n = edits.len();
    vec![correction(
        format!("tie@{line}"),
        "tie",
        line,
        format!(
            "an English song has {n} {} after a word (ties are for Czech and Slovak): make {} a space",
            if n == 1 { "~" } else { "~s" },
            if n == 1 { "it" } else { "them" }
        ),
        edits,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tex::fit::{apply, proposals};

    fn song(lang: &str, parts: &[(&str, &[&str])]) -> String {
        let mut out = format!("\\beginsong{{T}}[\n    lang={{{lang}}},\n]\n\n");
        for (head, lines) in parts {
            let kind = head.trim_start_matches("\\begin").trim_end_matches('*');
            out.push_str(&format!("{head}\n"));
            for line in *lines {
                out.push_str(&format!("    {line}\n"));
            }
            out.push_str(&format!("\\end{kind}\n\n"));
        }
        out.push_str("\\endsong\n");
        out
    }

    fn checked(text: &str) -> Vec<Proposal> {
        proposals(text)
            .into_iter()
            .filter(|p| p.is_correction())
            .collect()
    }

    const TYPED: [&str; 2] = [
        "\\[F]Tomorrow I'll \\[G]still be waiting",
        "\\[C]and so am I",
    ];
    const RETYPED: [&str; 2] = [
        "\\[F]Tomorrow Ill \\[G]still be waiting",
        "\\[C]and so am I",
    ];

    #[test]
    fn a_copy_differing_by_a_slip_takes_the_other_form() {
        let text = song(
            "english",
            &[("\\beginchorus", &TYPED), ("\\beginchorus", &RETYPED)],
        );
        let found = checked(&text);
        assert_eq!(found.len(), 1, "{found:?}");
        let p = &found[0];
        assert_eq!(p.id, "typo@11");
        assert_eq!(p.category(), "typo");
        assert_eq!(p.grade(), "correction");
        assert_eq!(
            p.description,
            "the 2nd chorus sings \"Ill\" where the 1st chorus sings \"I'll\" (line 6): make it \"I'll\""
        );
        let out = apply(&text, &[p]).unwrap();
        assert_eq!(out, text.replacen("Tomorrow Ill", "Tomorrow I'll", 1));
    }

    #[test]
    fn the_commoner_form_is_the_right_one() {
        let text = song(
            "english",
            &[
                ("\\beginchorus", &RETYPED),
                ("\\beginchorus", &TYPED),
                ("\\beginchorus", &RETYPED),
            ],
        );
        let found = checked(&text);
        assert_eq!(found.len(), 1);
        assert!(found[0].description.ends_with("make it \"Ill\""));
    }

    #[test]
    fn parts_differing_in_several_lines_differ_on_purpose() {
        let first = ["\\[C]Ty se mnou chodí lesem", "\\[G]a nikdo neví kudy"];
        let second = ["\\[C]Ty se mnou chodíš lesem", "\\[G]a nikdo nevíš kudy"];
        let text = song(
            "czech",
            &[("\\beginchorus", &first), ("\\beginchorus", &second)],
        );
        assert!(checked(&text).is_empty());
    }

    #[test]
    fn a_chord_inside_the_word_leaves_the_fix_to_a_person() {
        let split = [
            "\\[F]Tomorrow I\\[G]ll still be waiting",
            "\\[C]and so am I",
        ];
        let text = song(
            "english",
            &[("\\beginchorus", &TYPED), ("\\beginchorus", &split)],
        );
        let found = checked(&text);
        assert_eq!(found.len(), 1);
        assert!(found[0].edits.is_empty());
        assert!(found[0].description.ends_with("fix it by hand"));
    }

    #[test]
    fn apostrophes_are_made_one_kind() {
        let lines = ["It’s me, it’s you", "and it's late"];
        let text = song("english", &[("\\beginverse", &lines)]);
        let found = checked(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].description,
            "the song types apostrophes both ways, 1 ' and 2 ’: make them all ’"
        );
        let out = apply(&text, &[&found[0]]).unwrap();
        assert!(out.contains("and it’s late"));
    }

    #[test]
    fn ties_in_an_english_song_become_spaces() {
        let lines = [
            "\\lrep~\\[C]Walk a~\\[G]mile in my~shoes \\rrep",
            "Ho ~~~ hey",
        ];
        let text = song("english", &[("\\beginverse", &lines)]);
        let found = checked(&text);
        assert_eq!(found.len(), 1);
        let out = apply(&text, &[&found[0]]).unwrap();
        assert!(
            out.contains("\\lrep~\\[C]Walk a \\[G]mile in my shoes \\rrep"),
            "{out}"
        );
        assert!(out.contains("Ho ~~~ hey"));
        // In Czech they are right where they are.
        let text = song("czech", &[("\\beginverse", &["Jdu s~tebou v~lese"])]);
        assert!(checked(&text).is_empty());
    }

    #[test]
    fn words_leave_chords_and_macros_out() {
        let words = words_of(
            "\\[C]Tomorrow I'll \\[G]still be wait\\[D]ing, \\enquote{yes}",
            0,
        );
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Tomorrow", "I'll", "still", "be", "waiting", "yes"]);
        assert!(words[1].whole);
        assert!(!words[4].whole);
        assert_eq!(&"\\[C]Tomorrow I'll"[words[1].span.clone()], "I'll");
    }

    #[test]
    fn slips_are_one_letter_an_apostrophe_or_diacritics() {
        assert!(looks_like_a_slip("ill", "i'll"));
        assert!(looks_like_a_slip("ze", "že"));
        assert!(looks_like_a_slip("crying", "cryng"));
        // Short words differ by a letter on purpose: "the"/"she".
        assert!(!looks_like_a_slip("the", "she"));
        assert!(!looks_like_a_slip("love", "hate"));
    }
}
