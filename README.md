# Chords converter

## Usage
The first argument is the input source, the second (optional) is the output
format (`text`, `latex`, `debug`; defaults to `text`). Output is written to
`out/<format>/<artist>/<title>.<ext>`.

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

### Supported websites
| Site | Metadata read besides the song |
| --- | --- |
| `akordy.kytary.cz` | capo |
| `pisnicky-akordy.cz` | key |
| `blivnik.cz` | capo, chord diagrams, the date the sheet was added, the writer's remark |
| `ultimate-guitar.com` | key, capo, tuning, a tempo or time signature the tabber's remark names, the remark itself |

However differently the four sites store a song, underneath it is the same
"chords over lyrics" layout — so each parser only undoes what its site did to
it (its markup, its brackets, its JSON) and hands the sheet to the text parser,
which is where the chords, the sections and the tablature are read. What one
parser learns to read, all four can.

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