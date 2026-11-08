# chords_converter

Fetches chord sheets from the web (or reads text / saved pages) and writes them
as songs for the songbook in `../ZubuvZpevnik`. A converted song is a **draft**:
it always gets edited by hand before it goes into `songs/chords/`. This file
says what the finished song looks like, so the LaTeX printer can get as close
as it can and the hand edits can finish the job.

The macro reference is `../ZubuvZpevnik/zubuvzpevnik/README.md`; look things up
there rather than guessing. The website generator in `../ZubuvZpevnikWeb`
re-implements the macros, so a new macro is a change in both places.

The first half of this file is about the converter (the Rust code here). The
second half, from "Where a song goes" on, is the spec for the finished song.

## The converter

```bash
cargo build && cargo test          # unit tests live in-module, #[cfg(test)]
cargo clippy && cargo fmt
cargo run -- <url|file.html|file.txt|-> [latex|text|debug]
                                   # writes out/<format>/<Artist>/<Title>.<ext>
```

`out/` and `target/` are gitignored. `README.md` is the user-facing manual
(sites, text input format, bot checks). Keep it in step with the code.

### How it is built

- **`src/models.rs`**: `Song` → `Part` (`PartType`) → `Line` (lyrics +
  `PlacedChord`s at a character column) → `Chord` (root, modifier, bass).
  "N.C." is a chord of its own (`NO_CHORD`, `Chord::is_no_chord`), so it keeps
  its column. Metadata the model has no field for goes to `Song::extra`.
- **`src/parsers/`**: every site is "chords over lyrics" underneath. A site
  parser only undoes its site's markup, brackets or JSON, rebuilds the plain
  sheet and hands it to **`parsers/text.rs`**. That is where chords, sections,
  markers, tablature and the header are read. Fix reading problems there, so
  every site gains from the fix.
  - `get_parser` picks the parser by URL; `sniff_parser` recognises a page
    saved from a browser by its markup; `source_url` recovers the page's
    address from `canonical`/`og:url`.
  - Adding a site means a new module, an entry in **both** `get_parser` and
    `sniff_parser`, and a row in the README site table (the `add-site` skill).
- **`src/printers/`**: `get_printer` registry. `latex.rs` is the one that
  matters. `text.rs` is the inverse of `parsers/text.rs`, so text output read
  back in must give the same song (a good test invariant).
- **`src/main.rs`**: fetching (browser-like headers, cookie jar, retries,
  bot-check detection, `CHORDS_COOKIE` / `CHORDS_USER_AGENT`), file/stdin
  input, and choosing the output path.

### Rules for the code

- **Web content is untrusted.** Chord names reach LaTeX unescaped inside
  `\[...]`, and a build may run with `-shell-escape`. Every chord goes through
  `chord_safe`, everything set into a `%` comment goes through `one_line`, and
  lyrics go through `escape`. Any new path from page text to output has to be
  sanitised the same way, with a test that a hostile string can't escape.
- A chord keeps the **column** it was written in through every
  transformation (renaming `Emi`→`Em`, blanking markers, dedenting), because
  the column is the syllable it is sung on.
- Doc comments and the README explain *why*, in full sentences (look at
  `parsers/mod.rs`, `printers/latex.rs`). Write new ones the same way.
- Each behaviour change gets a unit test next to the code, built from a
  small hand-written snippet. Don't commit whole scraped pages.
- A change to what the LaTeX printer emits is checked against the songbook
  (the `printer-gap` skill), not just against unit tests.

### How the converter improves

About 80 finished songs in `../ZubuvZpevnik/songs/chords/` name a supported
site in `src=`. Each one is a target: convert the `src`, diff the draft
against the finished song, and every difference the printer could have made
itself is work for the converter. The section **"Printer output vs. the
finished song"** below is the roadmap. When a gap closes, delete or narrow its
bullet in the same change. Some differences need human judgement and are not
printer work: `met`/`tem` looked up on the web, fixing a site's wrong chords
or typos, choosing `\chordsoff` placement, fitting A5.

Project tooling in `.claude/`:

| | |
|---|---|
| skill `convert-song` | URL or saved page → draft → finished song in the songbook, checked |
| skill `printer-gap` | diff drafts against finished songbook songs, rank the gaps the printer could close |
| skill `add-site` | support a new chord site |
| agent `song-reviewer` | independent review of a `.tex` song against this file and the check scripts |
| agent `metadata-researcher` | look up meter, tempo, key and language on the web |

## Where a song goes

`../ZubuvZpevnik/songs/chords/<Artist>/<Title>.tex`: CamelCase, no diacritics,
no spaces (`KarelKryl/BratrickuZavirejVratka.tex`). It is `\input` from a
project file under `projects/`. One song per file.

## The prelude

```latex
\beginsong{She's Always A Woman}[
    by={Billy Joel},
    lang={english},
    tem={173},
    met={3/4},
    tun={Standard},
    key={E\flt},
    cap={3},
    src={https://...},
    chr={
        \gtab{F\shrp dim7}{2:(123131):123141}
    },
    pap={a4,a5},
]
```

- **`lang`** is always set (`czech`, `slovak`, `english`, …). It picks the
  hyphenation and the chord naming: Czech and Slovak songs use **H** for B
  natural and **B** for B flat, everything else uses English names.
- **`met`** goes on every song, looked up from sources on the web, never
  derived from the tab itself. Use one meter for the whole song wherever one
  fits (a 4/4 song with odd 2/4 bars is `met={2/4}`). Only a song no single
  meter can carry gets `\meter{n}{d}` in the body, and then no `met` key at all.
- **`cap`**, same rule: a capo that moves mid-song has no `cap` key, and
  every `\capo{n}` stays in the body.
- **Chords are the shapes you finger.** If the source wrote the sounding
  chords for a capo song, add `\transpose{-n}` after the prelude rather than
  rewriting every chord.
- **`key`** is the key the song *sounds* in (fingered shapes + capo +
  `\transpose` + a uniform detuning), spelled in the song's language (`Hm` in
  Czech, `Bm` in English), with `\flt`/`\shrp` (`key={A\flt m}`), never a bare
  `b`, `♭` or `\flat`. The space after `\flt` ends the control word: it is
  required before a letter (`\flt m`) and harmless before `}`. A song in two keys is
  `key={A\rightarrow Hm}`. `scripts/checkkeys.py` checks it.
- **`pap`** is measured, never written by hand:
  `scripts/checkpaper.py --measure a4 --write` and `--measure a5 --write`.
- A key the converter could not fill stays commented out (`% cap={},`) so it
  is easy to spot.

## Lines and chords

- **One source line is one printed line**, and every line of a song has the same
  musical length (all 4 bars, or all 2). Sites often split a phrase in two; join
  them back. Never break just the one line that is too wide.
- **Indent** part content by 4 spaces (`latexindent -l latexindent.yaml` in the
  songbook does it; it is rendering-neutral).
- **`\[X]`** goes immediately before the syllable the chord falls on.
  Sharp is `#`, flat is `&` (`\[F#m]`, `\[B&]`). Trailing chords after the last
  word are separated by single spaces: `\[E&sus4] \[E&]`.
- **No chord** is `\[\nochord]`, never `\[N.C.]` (its C would be transposed).
- **No `{…}` groups and no `~` to hold words under a chord.** Multiword chords
  are the package default: a wide chord hangs over the following words by
  itself. Do not emit `\MultiwordChords` either; it is the default now.
  `\SinglewordChords` exists only for a song that was measured to look worse
  with multiword chords.
- **`~` is only for Czech/Slovak typography:** after one-letter prepositions
  and conjunctions (`v~lese`, `a~když`, `s~tebou`) and the user's own
  preposition ties (`na~`, `do~`, `za~`, …). Never in English songs.
- Braces that *are* needed stay: `\[Am]{\multivoice{main}{second}}` puts a
  chord over a two-voice span.

## Song parts

| Part | When |
|---|---|
| `\beginverse … \endverse` | numbered verse |
| `\beginverse* … \endverse` | unnumbered verse-like line (e.g. a tag line) |
| `\beginchorus … \endchorus` | chorus, printed `R:` |
| `\beginprechorus` / `\beginbridge` | `P:` / `*` |
| `\beginintro` / `\beginoutro` | intro/outro **that has lyrics** |
| `\begininstrumental … \endinstrumental` | **anything with chords and no lyrics** |

**Chords-only passages become instrumentals.** An intro, outro, interlude or
solo that is just chords is a `\begininstrumental`, never a verse or an outro
of chord names. It is written in bars, one fixed-width box per slot:

```latex
\begininstrumental
    | \[E&]\[] | \[E&sus4]\[] | \[E&]\[] | \[B&]\[] |
    | \[Cm]\[] | \[\%]\[] | \[A&maj7]\[] | \[B&]\[] | \rep{2}
\endinstrumental
```

`\[]` is an empty slot that keeps the rhythm, `\[\%]` repeats the previous bar,
and the bars of a passage usually have the same number of slots (two per bar
is the common choice). Name parts by what they are in the song, not by the
site's label: the refrain of She's Always A Woman came from the site as a
"bridge".

## Repeats: write things once

- **A chorus that comes back is never retyped and never commented out.**
  `\insertchorus` reprints it whole; `\insertchorus*` prints the one-line
  reminder `R: first line...` (chords off, trailing punctuation dropped, `...`
  added). A following `\rep{n}` joins the reminder line: `\insertchorus* \rep{2}`.
- **Two different choruses** are numbered: `\beginchorus[1]`,
  `\beginchorus[2]`, repeated by `\insertchorus*[1]`. They print `R1:`/`R2:`.
  A chorus that comes back with new words is still the same chorus: no number.
- Every `\beginchorus` becomes "the latest chorus", even a hand-written
  partial one, so an `\insertchorus*` after it repeats *that*.
- A repeated line is `\lrep~… \rrep`, a repeated part or line ends with `\rep{n}`.
- A verse that repeats the first verse's progression keeps its `\[chords]` in
  the source but hides them: `\beginverse\chordsoff` (hidden everywhere) or
  `\beginverse\chordsoffonsmallpage` (hidden only on A5). **The first verse
  always shows its chords.** Spread the chorded verses evenly, so every plain
  verse has a chorded one next to it. Both switches also work mid-part, on a
  line of their own (`\chordson` turns chords back on).
- `^` replays the chords memorised from the previous part of the same type
  (each part type has its own memory); `\usechords{verse}` borrows another
  type's.

## Two voices and labels

A backing or second vocal line is `\multivoice{main}{second}` and stays a second
voice: never split it into its own lyric line, even to make a song fit.
`\role{Name}` labels who sings a line.

## Fitting the page (A5)

Most songs need nothing. For one that overflows A5, use these levers in order:

1. `\chordsoffonsmallpage` on parts that reprint chords already shown;
2. re-breaking lines, but only if *every* comparable line can be re-broken the
   same way;
3. `\smalleronsmallpage` (bare form only, 8 pt; anything smaller is too small
   to play from).

When a line has to be shortened, comment the tail out instead of deleting it.
Commented-out lines in songs are intentional and stay.

## Printer output vs. the finished song

What the LaTeX printer does not do yet, and so what the hand edit still has to
(the diff between `out/latex/BillyJoel/SheSAlwaysAWoman.tex` and the song in the
songbook shows all of it):

- chord-only parts keep the bars the source drew; a row drawn without bar
  lines comes out as slots with no bars (`\[Em]\[G]\[C]\[D]`), and the bar
  lines, which the page doesn't give, are the hand edit's;
- lines are as short as the site split them, not joined into whole phrases;
- repeated choruses are printed in full (`--tidy` makes an identical one
  `\insertchorus`), and repeated verses keep their chords. `\insertchorus*`
  and `\chordsoff` are a person's decision, which `suggest` proposes;
- without `--tidy`, chords sit where the site put them, often a letter inside
  the syllable (`D\[E]o` for `\[E]Do`);
- `met`, `cap`, `key` are left commented out when the site did not give them,
  and so is `lang` everywhere but on akordy.kytary.cz.

The plan for closing these is `docs/PLAN.md`: the printer does what is always
right (tier 1), `--tidy`/`tidy` makes the safe rewrites (tier 2), and
`suggest`/`apply` propose the changes a person decides on (tier 3).

## Checking a song

From `../ZubuvZpevnik`:

```bash
scripts/checksongs.py songs/chords/Artist/Song.tex   # metadata and syntax
scripts/checkkeys.py songs/chords/Artist/Song.tex    # silent when key= matches the chords
scripts/checkpaper.py --measure a5 --write songs/chords/Artist/Song.tex
projects/velkyzpevnik/build.sh                       # a real build (-halt-on-error)
```

For changes that should not alter the output (cleanups, reformatting), compare
the rendered pages before and after, not the sources.
