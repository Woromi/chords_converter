---
name: printer-gap
description: Measure how far the converter's LaTeX drafts are from the finished songs in ../ZubuvZpevnik, by converting the src= of songbook songs and diffing, then rank the differences the printer could close itself. Use to pick the next converter improvement, or to check that a printer/parser change moved drafts closer to finished songs (and broke none).
---

# Printer gap: drafts vs. finished songs

The finished songs in `../ZubuvZpevnik/songs/chords/` whose `src=` points at a
supported site are a regression and target corpus. This skill only measures
and proposes. Implementing a fix is a normal code change afterwards, with
unit tests.

Work in the session scratchpad directory (call it `$W`), not in the repo.

## 1. Build the corpus (fresh every time; don't hard-code it)

```bash
cd ../ZubuvZpevnik
grep -rlE 'src=\{https?://[^}]*(akordy\.kytary\.cz|pisnicky-akordy\.cz|blivnik\.cz|ultimate-guitar\.com|guitartuna\.com)' songs/chords
```

Take the site list from `get_parser` in `src/parsers/mod.rs`, in case it
has grown. For a quick look pick 3–5 songs across sites. For a before/after
measurement of a change, use a fixed sample of 10–20 and keep the list in
`$W/sample.txt` so both runs use the same songs.

## 2. Fetch once, convert many times

Pages change and sites throttle, so cache them in **`out/pages/`**
(gitignored, and it outlasts the session, so a before/after comparison across
sessions sees the same pages). The converter recognises saved pages by their
markup (`sniff_parser`) and reads `src` back from `canonical`/`og:url`:

```bash
mkdir -p out/pages
[ -s out/pages/<Artist>-<Title>.html ] || \
  curl -sL -A 'Mozilla/5.0' '<src url>' -o out/pages/<Artist>-<Title>.html
cargo run -q -- out/pages/<Artist>-<Title>.html latex
```

A page cached this way converted the same as a live `cargo run -- <url>`
(checked on ultimate-guitar and kytary). If a cached file is a bot check
(small, or the converter says so), delete it and use the live `cargo run` for
that song, or skip it and say so. Never commit scraped pages.

**Take the draft's path from the `Saving output to:` line.** The converter
names it from the page's artist and title, which need not match the
songbook's file name (diacritics, "feat.", duets).

**Before/after runs:** every run overwrites `out/latex/…`. Before changing
the code, copy the sample's drafts to `$W/before/`, then compare both
generations against the songbook.

## 3. Diff with the noise taken out

```bash
norm() { sed -E 's/^[[:space:]]+//; s/[[:space:]]+/ /g; /^$/d'; }   # reads stdin
diff <(norm < out/latex/A/T.tex) <(norm < ../ZubuvZpevnik/songs/chords/A/T.tex)
```

(Don't write shell positional parameters in this file. Skill arguments are
substituted into them when the skill loads.)

Indentation is already a known gap. Don't let it swamp the diff. Also read
one raw diff, so you see the whitespace gaps (trailing-chord spacing) too.

## 4. Classify every hunk

| Bucket | Examples | Whose job |
|---|---|---|
| **printer** | instrumentals not in bars, lines not joined, repeated chorus printed in full, repeated verses keep chords, spacing of trailing chords, `lang` not guessed for a Czech site | converter |
| **parser** | chord on the wrong column, a part type misread, a marker left in the lyrics, metadata on the page not read | converter (usually `parsers/text.rs`) |
| **source** | site has the wrong chord or a lyric typo, the finished song corrected it | nobody, the site is wrong |
| **judgement** | `met`/`tem`, `\chordsoffonsmallpage` placement, `\iflargepaper`, `\smalleronsmallpage`, `\enquote`, `chr` diagrams, `pap` | human / other tools |

When unsure whether the printer could know something, ask whether the
information is in the `Song` it receives. If it isn't, it is not a printer gap.

## 5. Report

- A table: gap → bucket → how many corpus songs show it → an example
  (file:line from both sides).
- The roadmap list "Printer output vs. the finished song" in CLAUDE.md: add
  gaps you found that are missing from it, and point out bullets that no
  longer happen.
- **One recommendation**: the most frequent printer/parser gap that is
  mechanical enough to implement safely, with where in the code it lives.

When re-running after a change, report per song: closer / same / further
(count changed diff lines). A song that got *further* is a regression. Show it.
