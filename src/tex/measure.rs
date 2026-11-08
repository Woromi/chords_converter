//! Whether a song fits a page, as the songbook itself measures it:
//! `scripts/checkpaper.py`'s `measure()`, the reference, called as it is
//! rather than imitated. It only measures songs under a songbook's
//! `songs/chords`, and a song being fitted may be a draft that isn't in the
//! songbook yet, so the song is measured in a songbook of its own: a
//! temporary root that links the real one's package, images and songs
//! (which a song's LilyPond files are found among) and holds a copy of the
//! song. Nothing is written into the real songbook. LilyPond's engraving
//! cache, which the measurement keeps under `build/`, lives in this tool's
//! own cache directory, so a score is engraved once rather than on every
//! measurement.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What one paper made of the song.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    /// Pages the song took.
    pub pages: u32,
    /// The song's lines that came out too wide for the column.
    pub wraps: Vec<Wrap>,
}

/// A line too wide for the column. A chorus is typeset from the copy the
/// songs package keeps of it, where the line a line came from is lost: the
/// probe reports the chorus's first line whichever line it was (tested with
/// the third one too wide). So for a chorus only that one is too wide is
/// known.
#[derive(Debug, Clone, PartialEq)]
pub struct Wrap {
    /// The song's line, 1-based; `None` for a line of a chorus.
    pub line: Option<usize>,
    pub in_chorus: bool,
}

impl Measured {
    /// A song fits when it takes no more pages than it may and no line had to
    /// be broken, which is checkpaper.py's own definition.
    pub fn fits(&self, pages_allowed: u32) -> bool {
        self.pages <= pages_allowed && self.wraps.is_empty()
    }
}

/// Where a songbook keeps the song directory the measurement copies into.
const MEASURED_ARTIST: &str = "ZzChordsConverterMeasure";

const MEASURE_PY: &str = r#"
import json, sys
from pathlib import Path
root, scripts, papers = Path(sys.argv[1]), sys.argv[2], sys.argv[3:]
sys.path.insert(0, scripts)
import checkpaper
song = root / "songs" / "chords" / "ZzChordsConverterMeasure" / "Song.tex"
name = "ZzChordsConverterMeasure/Song"
out = {}
for paper in papers:
    pages, wraps = checkpaper.measure(root, paper, [song])
    out[paper] = {"pages": pages.get(name), "wraps": [[part, line] for part, line in wraps.get(name, [])]}
print(json.dumps(out))
"#;

/// Measures `text` on each of `papers` (`a4`, `a5`) with the songbook at
/// `songbook`.
pub fn measure(
    songbook: &Path,
    text: &str,
    papers: &[&str],
) -> Result<BTreeMap<String, Measured>, String> {
    let songbook = songbook
        .canonicalize()
        .map_err(|e| format!("No songbook at '{}': {}", songbook.display(), e))?;
    let scripts = songbook.join("scripts");
    if !scripts.join("checkpaper.py").is_file() {
        return Err(format!(
            "'{}' has no scripts/checkpaper.py; --songbook has to name the ZubuvZpevnik checkout.",
            songbook.display()
        ));
    }

    let root = TempRoot::new()?;
    build_root(&songbook, root.path(), text)?;

    let output = Command::new("python3")
        .arg("-c")
        .arg(MEASURE_PY)
        .arg(root.path())
        .arg(&scripts)
        .args(papers)
        .output()
        .map_err(|e| format!("Could not run python3 to measure the song: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(25).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("Measuring the song failed:\n{}", tail.join("\n")));
    }

    let json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Unexpected answer from checkpaper.py: {}", e))?;
    let mut measured = BTreeMap::new();
    for paper in papers {
        let entry = &json[*paper];
        let pages = entry["pages"].as_u64().ok_or_else(|| {
            format!(
                "The song never reached the page on {}; is it a whole song?",
                paper
            )
        })?;
        let wraps = entry["wraps"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|w| {
                        let in_chorus = w[0].as_str()? == "r";
                        let line = w[1].as_u64()? as usize;
                        Some(Wrap {
                            line: (!in_chorus).then_some(line),
                            in_chorus,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        measured.insert(
            paper.to_string(),
            Measured {
                pages: pages as u32,
                wraps,
            },
        );
    }
    Ok(measured)
}

/// Lays out the temporary songbook: the real one's package, images and song
/// folders linked, `build/` of its own with the engraving cache linked in,
/// and the song copied.
fn build_root(songbook: &Path, root: &Path, text: &str) -> Result<(), String> {
    let io = |e: std::io::Error| format!("Could not prepare the measurement: {}", e);
    for linked in ["zubuvzpevnik", "img"] {
        let target = songbook.join(linked);
        if target.exists() {
            symlink(&target, &root.join(linked)).map_err(io)?;
        }
    }
    let chords = root.join("songs").join("chords");
    fs::create_dir_all(&chords).map_err(io)?;
    for entry in fs::read_dir(songbook.join("songs").join("chords")).map_err(io)? {
        let entry = entry.map_err(io)?;
        if entry.file_name() != MEASURED_ARTIST {
            symlink(&entry.path(), &chords.join(entry.file_name())).map_err(io)?;
        }
    }
    let own = chords.join(MEASURED_ARTIST);
    fs::create_dir_all(&own).map_err(io)?;
    fs::write(own.join("Song.tex"), text).map_err(io)?;

    let build = root.join("build");
    fs::create_dir_all(&build).map_err(io)?;
    let cache = cache_dir().join("tmp-ly");
    fs::create_dir_all(&cache).map_err(io)?;
    symlink(&cache, &build.join("tmp-ly")).map_err(io)?;
    Ok(())
}

/// This tool's cache directory: `$XDG_CACHE_HOME/chords_converter`, or
/// `~/.cache/chords_converter`.
fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("chords_converter")
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// A temporary directory, removed with everything in it when dropped. The
/// links in it are removed, never followed.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<TempRoot, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "chords_converter-measure-{}-{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path)
            .map_err(|e| format!("Could not create '{}': {}", path.display(), e))?;
        Ok(TempRoot(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // remove_dir_all removes symlinks without following them.
        let _ = fs::remove_dir_all(&self.0);
    }
}
