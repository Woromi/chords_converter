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