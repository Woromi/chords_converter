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

Proposals are rated by the readability they leave (see the README), then
by a cheap estimate of the printed lines saved. `--measure`
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
3. [x] **Song scanner** (`src/tex/mod.rs`): parts with byte spans, numbers,
   switches, `\if…\fi` depth, `\insertchorus…`, `lang`. Gate (`cargo test --
   --ignored`): all 216 songbook songs scan, every part re-slices.
4. [x] **Tier 2 `tidy`** and `convert --tidy` (`src/tex/`):
   - [x] Exact repeated chorus → `\insertchorus`. Gate: tidying the
         songbook changed 43 songs, and all 43 typeset pixel-identical
         before and after on A4 and A5.
   - [x] Chord moved back across consonants to its syllable's start (TeX
         patterns, minima 1/1, never across a vowel, never onto another
         chord, not in instrumentals or vowel-less words). Precision: 147 moves
         on the finished songbook. ~60 sampled were correct syllables once
         the hum guard (`mmm\[Hm]m`) and the collision guard (`\[G]p\[C]í`)
         were in. They are import leftovers the hand edit missed. Applies only
         where `lang` is set, so on drafts only to kytary songs for now.
         Recall: Samota's draft gap 41 → 24 lines, 21 Guns 100 → 87, Jolene
         37 → 32.
   - [x] Hand-typed reminder chorus → `\insertchorus*` (runs first). Gate:
         13 songbook songs compared, 10 pixel-identical once the songbook's
         `refren.lua` stopped keeping a trailing chord's space before the dots
         (fixed there, uncommitted). The 3 others (no dots: Hallelujah,
         Yomara Nildar; `\iflargepaper` inside: Mississippi Blues) are now
         left alone by rule. A `\chordsoff` on a line of its own (Sea of
         Machines, Basket Case) was first read as a sung line; once it was
         not, 25 more songs gained reminders, all pixel-identical.
   - [x] Idempotent: moves first, then choruses on the moved text; a second
         pass over every tidied songbook song changes nothing.
5. [x] **Tier 3 `suggest`/`apply`** (`src/tex/fit.rs`, `src/tex/measure.rs`):
   - [x] Proposals: `\chordsoff` / `\chordsoffonsmallpage` on a part repeating
         an earlier part's chords (the first keeps them, and chorded parts stay
         spread out, checked per paper and for proposals chosen together),
         `\insertchorus*` for a chorus with the same words (chords may differ,
         apostrophes ignored) or a full `\insertchorus`, and
         `\smalleronsmallpage` before the first part.
   - [x] `--measure`: the songbook's own `checkpaper.measure()` in a
         temporary root linking the songbook (no writes there), with the
         LilyPond cache in `~/.cache/chords_converter`. Fits = pages within
         the allowance (2 on A5 for `pap={…a5x2}`) and no line too wide.
   - [x] Precision: `suggest --fit a5 --measure` on all 198 songs marked for
         A5: 197 fit with nothing proposed. The one that doesn't
         (XindlX/VBlbymVeku, line 67 too wide) is confirmed by
         `checkpaper.py --measure a5`: its `a5` mark is stale.
   - [x] Spreading: a change may not add a break of the rule, neither for
         the part it touches nor for a neighbour it strands (counted before
         and after, per kind and paper). Every chord-hiding proposal on all
         216 songs (274), applied alone, checked by an independent script:
         no break added.
   - [x] A chorus is not shortened when a full `\insertchorus` after it
         reprints it (the reprint would go back to the chorus before).
   - [x] Too-wide lines: a verse's line number is right; a chorus's isn't
         known (the probe reports the chorus's first line), so only "a line
         of a chorus" is said.
   - [x] End to end on drafts: applying the first proposal and asking again
         stops. Forever And One gets the two `\insertchorus*` the hand edit
         chose. Africa runs out of proposals at 2 A4 pages: what the hand
         edit did there (joined lines, transposition) is not a lever here.
   - [x] Rated for readability (0.3.0, decided with the user 2026-11-08):
         a score and a grade per proposal, listed by score within a stage.
         A chorus reminder (80) reads better than chords off (70), which
         reads better than a smaller type (20). Chords off loses 2 per
         printed line between a plain part and the nearest part before it
         showing the same chords (the user: "minimize the distance"); ties
         go to the later part. Layouts are weighed whole, and a better one
         reached only by showing chords again is offered as one move
         (1 2 R 3- 4 R 5 → hide 2 and 4, show 3). `sources` names the
         parts a proposal relies on (asked for by the UI session).
   - [x] Lines joined in pairs (0.4.0, asked for by the user 2026-11-08):
         every two lines of every part of a kind into one, offered only when
         every part of the kind can take it (even number of lines; no
         comment, switch, `\textnote`, `\if` or inner `\rep` in the way)
         and, with `--measure`, the joined song typesets with no line too
         wide on A4 or A5 and no more pages. Rated 90: every word and chord
         stays. Offered in either stage, since it loses nothing on A4.
   - [ ] Re-breaking lines at other points stays a human judgement;
         `suggest` names the too-wide lines.
6. **Corpus checks (`printer-gap`).** Drafts move closer to the finished
   songs. `suggest` on *finished* songs proposes almost nothing, and every
   proposal there is a false positive to examine. (`tidy` on finished songs
   does find real work: import leftovers nobody fixed by hand.)

## Done once ZubuvZpevnik had it

- **A full chorus reprint with chords off.** The songbook spelled it
  `\insertchorus[n][setup]` (`[]` for the latest chorus). Tidy rule: a copy
  identical in words and chords to the latest chorus, turning only more
  chords off, becomes `\insertchorus[][\chordsoff]`. Gate: 21 songbook
  copies, pixel-identical on A4 and A5. With it, setup on lines of its own
  joins the opening line (140 lines, pixel-identical), so every rule finds
  the switches in one place. Still open: `suggest` could offer
  `\insertchorus` → `\insertchorus[][\chordsoffonsmallpage]` as a chords-off
  lever.

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
