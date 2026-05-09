use super::SongPrinter;
use crate::models::Song;
use std::io::Write;

pub struct DebugPrinter;

impl SongPrinter for DebugPrinter {
    fn print(&self, song: &Song, writer: &mut dyn Write) -> std::io::Result<()> {
        // writeln! takes the destination writer as its first argument
        println!("{:#?}", song);
        writeln!(writer, "{:#?}", song)
    }

    fn extension(&self) -> &str {
        "debug"
    }
}
