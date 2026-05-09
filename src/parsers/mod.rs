use crate::models::Song;

pub mod kytary_cz;

pub trait SongParser {
    // We now pass the exact link into the parse method
    fn parse(&self, url: &str, html_content: &str) -> Song;
}

pub fn get_parser(url: &str) -> Result<Box<dyn SongParser>, String> {
    if url.contains("akordy.kytary.cz") {
        Ok(Box::new(kytary_cz::KytaryCzParser))
    } else {
        Err(format!("No parser available for URL: {}", url))
    }
}