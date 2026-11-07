# Plan: songbook formatting

The converter produces songs closer to what goes into the songbook, and helps
make them fit a page. `../ZubuvZpevnikUi` drives it as a subprocess. The
converter owns this logic, and the UI only shows and applies results. The UI's
own `docs/PLAN.md` items for chorus `\chordsoff`, fit-to-page and `lang`
detection move here. The UI session updates its plan; this repo doesn't edit
the UI.

## Three tiers (decided with the user, 2026-11-07)

| Tier | What | Changes the PDF? | How it runs |
|---|---|---|---|
| **1. Always** | the printer's own output: `\enquote`, chord-only parts as instrumentals in bars, single spaces between trailing chords, 4-space indent, `lang` when known | it *is* the draft | every conversion |
| **2. Tidy** | automatic fixes behind a switch: an identical repeated chorus → `\insertchorus`, chords moved to the start of their syllable | `\insertchorus` no; syllables slightly | `convert --tidy`, and `tidy <song.tex>` |
| **3. Edit** | changes to what is printed: fit a page, shorten choruses, chords off | yes | separate commands only; proposals the user accepts or rejects |

**`\insertchorus` in tier 2 must be rendering-neutral.** It only fires when
the result typesets the same: a chorus repeated **exactly** (same lyrics, same
chords, ignoring whitespace). A near-identical one (Forever And One: `Ill`/`I'll`, one extra
chord) is a tier-3 proposal instead, because accepting it changes what is
printed. Checked by building before/after and comparing pages (CLAUDE.md:
"compare the rendered pages, not the sources").

**Tiers 2 and 3 edit text, not a model.** They work on an existing `.tex`,
which may have been edited by hand, so they scan it and replace byte spans.
Everything they don't touch stays byte-identical, and the song is never
printed again. `convert --tidy` prints the draft (tier 1), then runs the same
tidy code over the text, so there is one implementation.

## Command line

The UI runs `chords_converter <source> latex` and reads `Saving output to:
out/latex/<Artist>/<Title>.tex` (`ZubuvZpevnikUi/core/src/songimport.cpp`).
That form keeps working. Its output changes only through tier-1 improvements.

```
chords_converter convert <source> [--format latex] [--tidy] [-o FILE|-]
chords_converter tidy    <song.tex> [-o FILE|-]               # tier 2, applies all
chords_converter suggest <song.tex> [--fit a4|a5] [--kinds …] [--measure --songbook DIR] [--json]
chords_converter apply   <song.tex> --id ID… [-o FILE|-]      # tier 3, chosen proposals
```

- `suggest --json`: `{proposals: [{id, kind, description, gain_lines,
  edits: [{start, end, replacement}]}], measured?: {a4: pages, a5: pages}}`.
  Offsets are UTF-8 bytes of the file, and one proposal is one undo step in
  the UI. Ids are stable for the same text, so `apply --id` can follow
  `suggest`. The first proposal is the recommended next step.
- A failed write exits non-zero (today it exits 0).

## Fitting (tier 3), in two stages

1. **A4** with edits that act on every paper size (if A4 overflows, A5 does
   too): `\insertchorus*` for a repeated chorus, `\chordsoff` on verses
   repeating verse 1's chords (verse 1 keeps them, chorded verses spread
   evenly), near-identical chorus → `\insertchorus`.
2. Once A4 fits, edits **for small paper only** until A5 fits:
   `\chordsoffonsmallpage` on parts reprinting chords already shown, then
   `\smalleronsmallpage` (CLAUDE.md "Fitting the page").

Proposals are ranked by a cheap estimate (printed lines saved). `--measure`
asks the songbook's `scripts/checkpaper.py --measure a4|a5` for the real page
count, never with `--write`. The UI may use its own preview instead.
`checkpaper.py --measure` only takes songs under `songs/chords/`, so a draft
is measured in a scratch copy of the songbook (15 MB without `build/` and
`.git/`), never by writing into the real one. That works: a probe there
names the source lines that wrap (`--keep`, `paperprobe.wrap`).

## Chords inside a syllable (tier 2)

`hl\[E7]ubokou` becomes `\[E7]hlubokou`: the chord **moves to the start of
the syllable it is in** (decided with the user). Sites are often a letter off
(kytary's Samota: `D\[E]o tváře`). Syllables come from the **TeX
hyphenation patterns** for the song's `lang` (the `hyphenation` crate embeds
them), so the boundaries match what LaTeX itself uses. A song with no `lang`
is left alone. This changes where a chord prints, by design, so it is not
part of the identical-output check `\insertchorus` has.

## Milestones

1. [x] **CLI.** `convert` with `--format` and `-o FILE|-`, the legacy form
   unchanged, exit codes (1 failed, 2 bad usage), a saved bot check refused.
   `tidy`/`suggest`/`apply` come with their milestones.
2. [x] **Tier 1 in the printer.** Every chord-only part (not tablature) is an
   instrumental in bars: the source's `|` bars kept, otherwise one chord per
   bar, at least two slots, and `x2` → `\rep{2}`. Trailing chords one space
   apart, 4-space indent, `lang` from kytary's sheet (`cs`/`sk`/`en` only).
   Open: `lang` from the lyrics for the other sites.
3. **Song scanner.** Parts, lines and chords with byte spans. Unknown
   constructs are skipped and never rewritten. Gate: all ~200 songbook songs
   scan, and spans re-slice to the exact source.
4. **Tier 2 `tidy`.** Exact repeated chorus → `\insertchorus`. Verified on
   the corpus by building before/after and comparing rendered pages.
5. **Tier 3 `suggest`/`apply`.** Fitting stages, then syllables.
6. **Corpus checks (`printer-gap`).** Drafts move closer to the finished
   songs. `suggest`/`tidy` on *finished* songs propose almost nothing, and
   every proposal there is a false positive to examine.

## Notes for the UI session
- Exit codes changed: 1 when the song could not be read or written (it used to
  exit 0 even then), 2 for a wrong command line. The UI's CLAUDE.md says the
  converter "exits 0 even when writing failed". That is no longer true.
- `convert <source> -o -` prints the song to stdout, and `-o FILE` writes it
  where the UI wants it. Either can replace the temp directory and parsing
  the `Saving output to:` line, which still works as before.
- A saved bot-check page is refused (exit 1) instead of turning into
  `UnknownArtist/UnknownTitle.tex`. An unsupported URL no longer dumps the
  page's HTML to stdout.
- Drafts changed: indented part content, chord-only parts as instrumentals in
  bars, `lang` on kytary songs, and Czech chord names made English on
  English kytary songs. The captured outputs in `tests/data/converter/` are
  older than this.
- Output uses only macros in the UI's `knownSongCommands`, except
  `\textquotedbl` (from the stray-quote fix, commit 2cac66e). Add it there.
- Rebuild the converter (`cargo build --release`) after changes.
