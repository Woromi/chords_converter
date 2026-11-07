---
name: convert-song
description: Convert a chord sheet (URL, saved HTML page, text file) into a finished song for the ZubuvZpevnik songbook - run the converter, do the hand edits CLAUDE.md describes, place the file under ../ZubuvZpevnik/songs/chords/ and run the songbook's checks. Use when the user gives a song URL or says "add this song to the songbook".
---

# Convert a song into the songbook

The input is a URL, a saved page, a text file or pasted text. The result is
`../ZubuvZpevnik/songs/chords/<Artist>/<Title>.tex`, finished to the spec in
`CLAUDE.md` ("Where a song goes" onwards). That spec is the rulebook. Read it
again now and don't work from memory. This skill writes into **another
repository**. Don't commit there unless the user asks.

## 1. Draft

```bash
cargo run -q -- '<url or file>' latex     # → out/latex/<Artist>/<Title>.tex
```

- A bot check (always on pisnicky-akordy.cz, sometimes on ultimate-guitar):
  ask the user to save the page from the browser (Ctrl-S) and pass the file.
  See README "When a site answers with a bot check". Don't retry in a loop.
- Read the stderr warnings (sanitised chords, unknown metadata).
- If `songs/chords/<Artist>/<Title>.tex` already exists, stop and ask. It
  may be a different version, and it holds hand edits.

## 2. What only a human (or the web) knows

Start the `metadata-researcher` agent in the background right away, with
title, artist and the draft's chords. It returns `met`, `tem`, `key` and
`lang` with sources. Meanwhile:

- **Check the draft against the source page.** Sites are often wrong:
  chords on the wrong syllable, typos in the lyrics, a missing capo. If you
  know the song, say what looks off. Don't silently "fix" lyrics.
- `cap`: the shapes you finger. If the site gave sounding chords for a capo
  song, add `\transpose{-n}` and don't rewrite the chords.

## 3. Hand edits

Work down the "Printer output vs. the finished song" list in CLAUDE.md. That
is exactly what the draft still lacks:

1. Prelude: `lang`, `met`, `tem`, `key` (sounding key, in the song's
   language) filled in or still commented out. Remove a commented key only
   once it's filled.
2. Join split lines so **every line has the same musical length**.
3. Chord-only parts already come as `\begininstrumental` in bars. Fix the
   rhythm: a row the site drew without bar lines is one chord per bar, which
   may need two chords to a bar, or `\[\%]` for a repeated one.
4. Repeated chorus → `\insertchorus` / `\insertchorus*`, never retyped.
5. Verses repeating verse 1's progression → `\chordsoff` /
   `\chordsoffonsmallpage`, spread evenly. Verse 1 keeps its chords.
6. `\[\nochord]`; trailing chords separated by single spaces; Czech/Slovak
   `~` ties; no `{…}`/`~` used to hold words under a chord.
7. The draft comes indented. After hand edits, re-indent (docs/building.md in
   the songbook). Don't use `-w`, which leaves
   `.bak` files beside the song, and don't use `-s`, which suppresses the
   output. Write to the scratchpad, keep the log there too, then move it in:
   ```bash
   cd ../ZubuvZpevnik
   latexindent -l latexindent.yaml -c "$SCRATCH/" -o "$SCRATCH/T.tex" songs/chords/A/T.tex >/dev/null \
     && [ -s "$SCRATCH/T.tex" ] && mv "$SCRATCH/T.tex" songs/chords/A/T.tex
   ```
   (`$SCRATCH` = the session scratchpad.)

## 4. Place and check

```bash
mkdir -p ../ZubuvZpevnik/songs/chords/<Artist>
cp out/latex/<Artist>/<Title>.tex ../ZubuvZpevnik/songs/chords/<Artist>/   # then edit there
cd ../ZubuvZpevnik
scripts/checksongs.py songs/chords/<Artist>/<Title>.tex
scripts/checkkeys.py  songs/chords/<Artist>/<Title>.tex          # silent = key fits
scripts/checkpaper.py --measure a4 --write songs/chords/<Artist>/<Title>.tex
scripts/checkpaper.py --measure a5 --write songs/chords/<Artist>/<Title>.tex
```

The file is an orphan until a project under `projects/` `\input`s it. Ask the
user which project(s). Don't guess. Then run that project's `build.sh`. It
uses `-halt-on-error`.

If A5 overflows, use the levers in CLAUDE.md "Fitting the page", in order.

## 5. Review and feed back

- Hand the finished file to the `song-reviewer` agent for an independent
  pass, then fix what it confirms.
- Look at what you had to do by hand. If an edit was mechanical (the printer
  could have done it), check that it is on the roadmap list in CLAUDE.md, and
  tell the user. That is how the converter improves (`printer-gap` skill).
