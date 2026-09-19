// Add Clone and Debug to all structs
#[derive(Clone, Debug)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub source_website: String,
    pub key: Option<String>,
    pub tempo: Option<String>,
    pub language: Option<String>,
    pub tuning: Option<String>,
    pub capo: Option<u8>,
    /// `Label: value` pairs the source provided that have no field of their
    /// own (year, album, writers, ...), kept in the order they were read.
    pub extra: Vec<(String, String)>,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug)]
pub enum PartType {
    Intro,
    Verse,
    PreChorus,
    Chorus,
    Bridge,
    Instrumental,
    Tab,
    Outro,
    Other(String),
}

#[derive(Clone, Debug)]
pub struct Part {
    pub part_type: PartType,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub lyrics: String,
    pub chords: Vec<PlacedChord>,
}

#[derive(Clone, Debug)]
pub struct PlacedChord {
    pub chord: Chord,
    pub position: usize,
}

#[derive(Clone, Debug)]
pub struct Chord {
    pub root: String,
    pub modifier: Option<String>,
    pub bass: Option<String>,
}

/// The mark a sheet puts where the music goes on with no chord under it —
/// "N.C.", for *no chord*. It is carried as a chord of its own, so that it
/// keeps the column it stands in and the line it stands on still reads as a
/// chord line; the printers know to set it as the rest it is rather than to
/// name it.
pub const NO_CHORD: &str = "N.C.";

impl Chord {
    /// The "no chord" mark, whatever spelling it was read in.
    pub fn no_chord() -> Chord {
        Chord {
            root: NO_CHORD.to_string(),
            modifier: None,
            bass: None,
        }
    }

    /// Whether this is that mark rather than a chord to play.
    pub fn is_no_chord(&self) -> bool {
        self.root == NO_CHORD
    }
}
