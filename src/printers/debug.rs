use std::io::Write;
use crate::models::Song;
use super::SongPrinter;

pub struct DebugPrinter;

impl SongPrinter for DebugPrinter {
    fn print(&self, song: &Song, writer: &mut dyn Write) -> std::io::Result<()> {
        // writeln! takes the destination writer as its first argument
        writeln!(writer, "{:#?}", song)
    }

    fn extension(&self) -> &str {
        "debug"
    }
}