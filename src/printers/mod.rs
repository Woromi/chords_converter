use crate::models::Song;
use std::io::Write;

pub mod debug;
pub mod latex;
pub mod text;

pub trait SongPrinter {
    // We now write to a generic destination stream and return an I/O Result
    fn print(&self, song: &Song, writer: &mut dyn Write) -> std::io::Result<()>;

    // Returns the preferred file extension for this format
    fn extension(&self) -> &str;
}

pub fn get_printer(format: &str) -> Result<Box<dyn SongPrinter>, String> {
    match format.to_lowercase().as_str() {
        "debug" => Ok(Box::new(debug::DebugPrinter)),
        "text" => Ok(Box::new(text::TextPrinter)),
        "latex" => Ok(Box::new(latex::LatexPrinter)),
        _ => Err(format!(
            "Unknown format '{}'. Available formats: text, latex, debug",
            format
        )),
    }
}
