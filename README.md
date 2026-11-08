# Chords converter

## Usage
The first argument is the input source, the second (optional) is the output
format (`text`, `latex`, `debug`; defaults to `latex`). Output is written to
`out/<format>/<artist>/<title>.<ext>`, and the path is printed as
`Saving output to: <path>`.

```
# From a supported website
cargo run -- <url> <output_format>

# From a local text file (chords over lyrics)
cargo run -- song.txt latex

# From a page saved in a browser
cargo run -- kometa.html latex

# Pasted directly into the console (finish with Ctrl-D)
cargo run -- - latex
```

`convert` does the same with options: `--format` names the output format,
and `-o FILE` writes the song to a file of your choosing, or `-o -` to
standard output, for a program that keeps the song itself:

```
cargo run -- convert <url> -o Song.tex
cargo run -- convert kometa.html --format text -o -
```

`--tidy` also makes the improvements below to the draft, and `tidy` makes
them to a song that is already in the songbook:

```
cargo run -- convert <url> --tidy
cargo run -- tidy ../ZubuvZpevnik/songs/chords/Hoboes/Samota.tex        # rewrites the file
cargo run -- tidy Song.tex -o -                                          # prints it instead
```

`tidy` changes only the bytes it means to and lists each change on stderr.
The file is rewritten only when something changed, and through a file beside
it, so it is never left half written.

- **Runs of spaces become one, and spaces ending a line go.** TeX reads
  them that way already. Indentation, comments (which may hold text lined
  up by hand) and the space of a control space `\ ` are kept. On the
  songbook: 152 lines in 49 songs, all typeset pixel for pixel the same.
- **A chord between two words gets a space before it**: `dál\[G] a sál`
  becomes `dál \[G] a sál`. Glued to the word before, the chord belongs to
  neither word, and with `\chordsoff` the line reads otherwise. Chords at
  the end of a line, inside a word, `^`, `\[]` slots and instrumentals stay
  as they are. On the songbook: 21 chords in 13 songs. The printer never
  writes it this way in the first place.
- **Setup on lines of its own moves onto the opening line**:
  `\beginverse` followed by a line `\chordsoff` becomes
  `\beginverse\chordsoff`. Only the lines before the first sung one move,
  and only part setup (`\chordsoff`, `\chordsoffonsmallpage`, `\memorize`,
  `\replay`, `\usechords`, `\copychords`, `\transpose`,
  `\smalleronsmallpage`, `\multivoiceoff`), never a line with a comment. TeX
  reads both the same; on the songbook 140 lines moved, all typeset pixel
  for pixel the same.
- **A chorus typed out again becomes `\insertchorus`**, when it is the same
  as the latest chorus, word for word and chord for chord, with the same
  number and the same switches after `\beginchorus`. `\insertchorus`
  reprints that chorus exactly: 43 songbook songs tidied this way typeset
  pixel for pixel the same, on A4 and A5. A song whose choruses sit inside
  `\iflargepaper … \fi`, or replay chords (`^`, `\memorize`), is left alone.
- **A copy that only turns the chords off becomes
  `\insertchorus[][\chordsoff]`** (or `[][\chordsoffonsmallpage]`, `[2][…]`
  for chorus 2): the second bracket runs inside the reprint. A copy showing
  chords the latest chorus hides stays typed out. On the songbook: 21
  copies (Hallelujah, Yomara Nildar, …), all typeset the same. It needs a
  songbook whose `\insertchorus` takes the second bracket.
- **A reminder typed out by hand becomes `\insertchorus*`**: a chorus of one
  sung line, chords off (`\chordsoff` on the opening line or on a line of
  its own before the words), ending in `...`, the rest commented out, with the
  words of the latest chorus's first line. That is what `\insertchorus*`
  prints. A reminder in words of its own, without the dots, depending on
  the paper (`\iflargepaper`), or showing chords stays as written. So does
  one that a full `\insertchorus` after it reprints. Words count as the
  reminder prints them, the main voice of a `\multivoice`. One typed without
  a two-voice label ending the first line (`\role{Ty}{Arjen}`, which the
  one-voice reminder would cut to `Ty`) becomes
  `\insertchorus*[][\rolesoff]` (Connect the Dots). One typed without a
  one-voice label stays: the label may have been forgotten, and putting it
  back shows on the page. On the songbook this
  replaced 57 reminders in 34 songs, all typeset pixel for pixel the same
  (with the songbook's `\insertchorus*` dropping a trailing chord before its
  dots, as it now does).
- **A chord drawn inside its syllable moves to the syllable's start**:
  `hl\[E7]ubokou` becomes `\[E7]hlubokou`, `D\[E]o` becomes `\[E]Do`.
  Sites draw a chord over the vowel and leave the consonants that open the
  syllable in front of it. Syllables come from the TeX hyphenation patterns
  for the song's `lang`. Those patterns don't know every syllable (English
  ones find none in "melodies"), so a chord only moves back across
  consonants, never across a vowel. This one does change where a chord
  prints, which is the point.

### Fitting a page: `shrink`, `join` and `apply`

Some changes alter what a song prints, so a person decides on them.
`shrink` lists them, rated and most readable first, and `apply` makes the
ones chosen:

```
cargo run -- shrink Song.tex --fit a5 --measure
cargo run -- apply Song.tex --id chordsoff@31          # then shrink again
cargo run -- join Song.tex                             # lines joined in pairs, checked
```

```
Song.tex
  A4: 1 page, fits
  A5: 2 pages, does not fit
  It fits A4: the changes for small paper only.
  Proposals by paper, the most readable first (score 0-100):
   for small paper only:
     60 fair        chordsoffonsmallpage@35      verse 3 repeats the chords of verse 1: \chordsoffonsmallpage (~6 lines)
     20 last resort smalleronsmallpage@13        set the song a size smaller on small paper (8 pt): \smalleronsmallpage (~7 lines)
```

- **Two stages.** A song that overflows A4 overflows A5 too, so it is first
  fitted to A4 with changes that act on every paper: `\insertchorus*` for a
  chorus sung again with the same words and chords, give or take one chord
  (or reprinted with `\insertchorus`; a chorus in another key keeps its
  chords, since the reminder prints none), `\chordsoff` on a part that repeats the chords of an
  earlier one. Once A4 fits, the changes act on small paper only:
  `\chordsoffonsmallpage`, then `\smalleronsmallpage` as the last resort.
- **Rating.** Each proposal has a score from 0 to 100 for how readable it
  leaves the song, and a grade: good (75 and up), fair (55), poor (35), last
  resort. Within a stage they are listed by score, then by the lines they
  save. Best is a song with all its words and all its chords, so every
  change costs something:

  | Change | Score | Why |
  |---|---|---|
  | join lines in pairs | 90 | every word and chord stays where it is sung |
  | an instrumental's row played again, once with `\rep{n}` | 85 | nothing looked up elsewhere, only counted |
  | `\insertchorus*` | 80 | one glance back to the chorus, and back |
  | `\chordsoff`, `\chordsoffonsmallpage` | 70 | words read here, chords somewhere else, line after line |
  | `\smalleronsmallpage` | 20 | the whole song harder to read |

  Where chords go off counts too. A part without chords is read with the
  chords of the nearest part before it that shows the same ones, so it
  loses 2 for each printed line between them (a full `\insertchorus`
  counting the whole chorus), on the paper where that is further, and for
  any other part it sends further back. A verse right below the one it
  repeats loses nothing. Of two as near, the later goes first, so chords
  stay towards the beginning: `V R V R V R` loses the last verse's chords,
  `V V R V V R V V R` the second verse of the last pair. Every layout with
  one more part without chords is weighed as a whole, and when the best
  one needs chords shown again somewhere, that is one proposal ("hide the
  chords of verse 2 and verse 4 and show them again in verse 3"), so taking
  the first proposal each time never leads into a corner. Small paper never
  gets more chords back than it loses. The saving is an estimate in printed
  lines, a chord row counting as one; a change that saves none is not
  offered.
- **Joining lines: `join`, a command of its own.** Every two lines of every
  part of a kind (all verses, say) become one: fewer, longer lines, all of
  one musical length again. It is offered only if every part of the kind
  can be joined alike (an even number of lines, and no comment, chord
  switch, `\textnote`, `\if…` or `\rep` before a part's last line in the
  way), and only once the joined song is typeset with no line too wide on
  A4 or A5 (nor one already). That is a typesetting run per kind of part,
  seconds on a long song, so `shrink` leaves it out and `join` does it on
  request. Its output is `shrink`'s, and its ids work with `apply`.
  Instrumentals join too, bar to bar: `| a | b |` and `| c | d |` become
  `| a | b | c | d |`, the bar line where they meet written once; one
  with a `\rep` row is left alone.
- **Rows played again.** An instrumental's row written several times in a
  row is printed once with `\rep{n}` (kind `rep`); a row that already ends
  with `\rep{k}` counts k times. Rows of `^` are left alone: each `^`
  replays the next chord remembered, so rows written alike play different
  chords. On the songbook: 17 instrumentals in 15 songs.
- **Chords stay spread out.** The first part of a kind keeps its chords, and
  a part that loses them must still have a chorded neighbour of its kind.
  This holds on each paper, for each proposal and for any chosen together.
- **`--measure`** typesets the song with the songbook's own
  `scripts/checkpaper.py` (the reference measurement, not imitated) and says
  how many pages it takes and which lines are too wide. A song fits when it
  takes one page (two for `pap={…a5x2}`) and no line is too wide; a too-wide
  line is re-broken by hand. The song is measured in a temporary copy of the
  songbook's layout, so a draft that isn't in the songbook yet can be
  measured too, and nothing is written into the songbook. The songbook is
  `../ZubuvZpevnik` unless `--songbook DIR` says otherwise. Once a song fits,
  record it with `scripts/checkpaper.py --measure a5 --write <song>` there.
- **Descriptions name the parts** as a person counts them: "verse 3" as the
  page numbers it, "the 2nd chorus" (a reprint with `\insertchorus` counts
  as sung again), "the 1st chorus R2" for a numbered one, and a part type a
  song defines for itself by its macro ("the 2nd `\beginbridgeb` part").
  The `line` field still gives the line in the file.
- **`--json`** gives the same for a program, each proposal with its `score`,
  its `grade`, its edits as UTF-8 byte offsets into the file, and `sources`:
  the parts it relies on (where hidden chords are still shown, the chorus a
  reminder stands for), each as its first line and its bytes from `\begin…`
  to the end of `\end…`.
- **`apply`** finds the proposals afresh in the file as it is, so an id from
  an older `shrink` that no longer applies is refused. Proposals that change
  the same text, or that together would leave a part without a chorded
  neighbour, are refused together; apply one, then shrink again.

### Corrections: `check`

`check` lists what may be wrong in a song, whether it fits a page or not,
in the same format as `shrink` (`--json` too), and `apply` makes the ones
chosen. Fitting and correcting are different actions, so neither lists the
other's proposals (`shrink` was called `suggest` until 0.6.0; that name
still works).

```
cargo run -- check Song.tex
```

- **A chorus typed as something else.** A part with the words of the
  latest chorus that isn't a chorus (a modulated chorus a site marked "key change" comes in as
  unnumbered verses) may be the chorus sung again. Only a person can tell
  (the same words need not be the same melody), so each such part is a
  proposal of its own, and it only makes the part a chorus, its chords
  kept. The rest is the other rules, a step at a time: `tidy` makes a
  chorus typed out again `\insertchorus`, and `shrink` then offers
  `\insertchorus*`. Its `category` in the JSON is `"restructure"`.
- **Typos, the song checked against itself** (no dictionary): a line sung
  more than once whose copies differ in one word, by an apostrophe,
  diacritics or one letter in a word of four or more (`Ill` beside `I'll`).
  The rarer form is offered the commoner one, and with as many of each, the
  fuller: with the apostrophe, the diacritics, or the longer word. Two
  parts that differ in more than one line differ on purpose (a chorus sung
  to "you" the second time) and are left alone. A word with a chord inside
  is named, but fixed by hand.
- **Apostrophes typed both `'` and `’`**: all made the commoner kind (TeX
  prints them alike; this keeps the source consistent).
- **`~` after a word in an English song**: a space. Ties are for Czech and
  Slovak; `\lrep~` and `~~~` for spacing stay.

Corrections have no readability score: their `score` is 0 and their
`grade` is `correction`, and their `category` is `restructure` or `typo`.
On the songbook, `check` finds five songs with mixed apostrophes and one
line to look at (Heart of Gold: "keep" beside "keeps").

The tool exits with 1 when the song could not be read or written and with 2
when the command line is wrong, so a caller need not read the messages to
know it failed.

### Supported websites
| Site | Metadata read besides the song |
| --- | --- |
| `akordy.kytary.cz` | capo, language |
| `pisnicky-akordy.cz` | key |
| `blivnik.cz` | capo, chord diagrams, the date the sheet was added, the writer's remark |
| `ultimate-guitar.com` | key, capo, tuning, a tempo or time signature the tabber's remark names, the remark itself |
| `guitartuna.com` | key, tempo, time signature, tuning, capo |

A site that says the song needs no capo (kytary's "pražec 0", blivnik's
`[CAPO:0]`, Ultimate Guitar's "Capo: no capo") leaves the draft without a
`cap` key; one that says nothing leaves `% cap={},` for a person to fill in.

However differently the five sites store a song, underneath it is the same
"chords over lyrics" layout — so each parser only undoes what its site did to
it (its markup, its brackets, its JSON) and hands the sheet to the text parser,
which is where the chords, the sections and the tablature are read. What one
parser learns to read, all five can.

GuitarTuna lays a song out by the beat rather than by the column, so there is
no sheet to read off the page as it stands. It does, however, write every chord
into the lyrics for screen readers, right where the chord is played — and that
is the column the parser writes it back out in.

Ultimate Guitar keeps the sheet in a JSON blob the page ships for its own
scripts, not in the HTML, so the parser reads it from there. The capo, the
tuning and the key are kept beside the sheet rather than in it — the site
prints them in the header above it, and a tab whose remark never mentions a
capo is played with one all the same — so the header is read first and the
tabber's remark only fills in what it left out. It also turns away
anything that does not look like a browser, which is why pages are fetched with
the headers one sends, a cookie jar and a couple of retries. Should a page still
come back as a bot check, open it in a browser, save it and pass the saved file:
a saved page is recognised by the markup its site is built on, so it needs no
address, and the one the page records is what the song names as its source.

### When a site answers with a bot check
pisnicky-akordy.cz stands its whole site behind a Cloudflare challenge, which it
serves with a 403 where Ultimate Guitar serves its own with a 200 — so it is the
body, not the status, that says a challenge came back instead of the song.

That challenge is answered by running its JavaScript, and no plain HTTP client
runs any: looking more like a browser does not help, and neither does looking
like one down to the TLS fingerprint — Chrome, Firefox and Safari fingerprints
are all turned away just the same. Retrying is therefore pointless too, so a
recognised challenge is reported at once rather than asked for three times over.

Two things do get past it, both by way of a browser that has already been let
in:

```
# 1. Save the page from the browser (Ctrl-S) and pass the file
cargo run -- page.html latex

# 2. Lend the tool the clearance the browser holds
export CHORDS_COOKIE='cf_clearance=<value>'
export CHORDS_USER_AGENT='<what navigator.userAgent prints>'
cargo run -- <url> latex
```

The cookie is in DevTools → Application (Storage) → Cookies, on that site.
Cloudflare ties it to the browser that earned it and to the address it was
earned from, so `CHORDS_USER_AGENT` has to be that same browser's, which
`navigator.userAgent` in the console prints. It lasts a while rather than
forever; when it lapses, copy it again or fall back on the saved page. Both
variables apply to every site, so a `cf_clearance` for one is best exported only
for the run that needs it.

### Text input format
Plain-text songs use the common "chords over lyrics" layout — a line of chords
positioned above the lyric line it belongs to. The header, `Label: value`
metadata block and section markers are all optional:

```
Beatles - Let It Be
===================
Key:   C
Capo:  0

[Verse]
       C          G
When I find myself in times of trouble

R:
       Am        G/B
Let it be, let it be
```

Sections can be marked with `[Brackets]`, with inline markers such as `R:`,
`Ref:`, `1.`, `Intro`, `Bridge`, `Outro` (Czech and English), or with the same
markers written in brackets of their own — `(1)`, `(R)`, `(I)`. A marker may
share its line with what follows it — both `1. When I find myself` and
`Intro: | C  G | Am  F |` are understood. A blank line ends the current block.
A block no marker named and with nothing to sing in it — a line of chords on
its own — is read as the intro when it opens the song and as an instrumental
part anywhere else.

The marker gives its columns up rather than its place: it is blanked out, so
the chords above it keep standing over the syllables they were written over.
The margin a whole block shares — the one a "1." only its first line carries
leaves behind — is then dropped from the block as a whole, chords and lyrics
together.

Both standard (`C#m7`) and Czech (`Cis`, `Dmi`, `As`) chord names are
recognised, with the bass note written either `C/E` or `C\E`. Bar lines and
repeat marks (`|`, `:||`, `x2`) may share a line with chords. `N.C.` — *no
chord*, the mark for a passage that goes on with none under it — is read as a
chord of its own, so the line it stands on still reads as a line of chords and
it keeps the column it was written in; however it is spelled (`N.C`, `NC`,
`N/C`), LaTeX gets it as `\[\nochord]`.

A minor chord is written out the short way whatever the source called it:
`Emi`, `Dmin` and `Em` all come out as `Em`, `Hmi7/Fis` as `Hm7/Fis`, and the
key the page names the song's is shortened with them. Only the "mi" goes — the
root is left as the source spelled it, so `Cis` stays `Cis` and `H` stays `H` —
and it only goes where it stands directly behind a root, which leaves the `mi`
of the lyrics alone. A chord keeps the column it was written in, so shortening
its name does not move it off the syllable it is sung on. Tabs are expanded
to 8-column tab stops, so a chord line indented with tabs still lines up with
the lyrics below it.

The header is only read when the first line is underlined with `===` or
followed by metadata. Every `Label: value` line in that block is kept: the
labels the song model knows (`title`, `artist`, `key`, `tempo`, `meter` or
`time signature`, `tuning`, `capo`, `language`, `source`, plus Czech
equivalents) fill in its fields, the
rest (`Year`, `Album`, `Writers`, ...) are carried through to the output — as
commented-out keys in LaTeX.

This is the inverse of the `text` printer, so its output can be fed straight
back in.

## Debug
```
cargo build
cargo run -- <url> <output_format>
```

## Install
```
cargo install --path .
```

## TODO: Cíl projektu
```
I dream of a large project of my personal songbook. The ultimate goal is to
- Have my personal songs database with chords. Songs should contain metadata, such as name, artist, key, tuning, tempo, used chord diagrams, original version of the song, source website. Then, it should contain songparts, which could be either instrumental (bars, chords), lyrics (verse, chorus, intro, outro, prechorus, bridge) or lilypond blocks
- Have a latex package improving an existing songs package, and implement two classes using my package. 
- Have multiple latex projects, creating separate documents, using my classes. I can choos songs for each of the document.
- Have lilypond files for some songs included. 

- Have a program (either Rust or C++) that can fetch chords from different websites, converting them to my format. It should then be able to print the song to the latex, and other formats.
- Other formats could be: html, some format that could be shown in the Android, iOS or desktop app.
- Possibly GUI in the future. 

Can you help me create some plan to implement this? So far, I have:
- Latex project. It would be good if it could be used in some form. The project has: zubuvzpevnik (implementing the latex package and class), songs (the songs, each in its own tex file, possibly with additional .ly files referenced from the .tex file), scripts (building the latex document, the index, cleanup, songidx.lua script), projects (documents with references to songs, that should be included). I also created different set of songs for tinwhistle (folders songs/chords and songs/tinwhistle) containing songs with just the lilypond file included.

I also have a sample rust program parsing some websites, able to create simple tex file. I still need to process each file manually, because most of the websites have no information about the song tempo, and also, the chord positioning is usually wrong.
```