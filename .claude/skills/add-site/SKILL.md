---
name: add-site
description: Add support for a new chord website to the converter - a parser module, URL and saved-page detection, tests and README. Use when the user wants to convert songs from a site get_parser doesn't know yet (e.g. kytaristka.cz, which songbook songs already cite).
---

# Add a chord site

Read `src/parsers/mod.rs` and the closest existing parser first.
`kytary_cz.rs` and `pisnicky_akody_cz.rs` are HTML scrapers,
`ultimate_guitar_com.rs` reads a JSON blob, `guitartuna_com.rs` rebuilds
columns from inline chords.

## 1. Look at the page

Get one or two real pages into the session scratchpad (`curl -sL -A
'Mozilla/5.0' <url> -o …`, or ask the user to save it from the browser if a
bot check answers). Find out:

- where the sheet lives: HTML markup, inline chord spans, a JSON blob, a
  `<pre>`;
- whether chords sit **above** lines (columns) or **inside** lyrics (then you
  rebuild the column, as in `guitartuna_com.rs`);
- the metadata on the page: title, artist, key, capo, tuning, tempo, meter;
- a landmark unique to the site's markup, for `sniff_parser`.

## 2. Write the parser: undo the site, then delegate

The parser's only job is to turn the page back into the plain "chords over
lyrics" sheet (with `[Verse]`/`R:` markers and a header if useful) and call
the text parser. Don't re-implement chord or section reading. If the text
parser can't read something the site produces, fix `parsers/text.rs` so all
sites benefit. Then fill in the `Song` fields the page gives beyond the sheet
(capo, key, …).

The page is untrusted input. Output sanitising happens in the printers, but
never build LaTeX in a parser.

## 3. Register it (every place)

- `pub mod` + a branch in `get_parser` (URL substring);
- a branch in `sniff_parser` (markup landmark). Make sure it doesn't also
  match the other sites' pages. Order matters;
- README: the "Supported websites" table row (metadata read), plus a
  paragraph if the site has a quirk (like the UG / GuitarTuna ones).

## 4. Test

- Unit tests in the module (`#[cfg(test)]`) on **small hand-trimmed
  snippets** of the real markup, never a whole saved page: title/artist,
  one chord line landing on the right syllable, metadata, sniffing.
- `cargo test && cargo clippy`.
- End to end: `cargo run -q -- <url> latex`, then from a saved file. Both must
  give the same `src=`.
- If songbook songs cite this site, run the `printer-gap` skill on them.
  That shows whether the new parser's output is as close as the other sites'.
