use crate::models::Song;

pub mod kytary_cz;
pub mod pisnicky_akody_cz;

pub trait SongParser {
    // We now pass the exact link into the parse method
    fn parse(&self, url: &str, html_content: &str) -> Song;
}

pub fn get_parser(url: &str) -> Result<Box<dyn SongParser>, String> {
    if url.contains("akordy.kytary.cz") {
        Ok(Box::new(kytary_cz::KytaryCzParser))
    } else if url.contains("pisnicky-akordy.cz") {
        Ok(Box::new(pisnicky_akody_cz::PisnickyAkordyCzParser))
    } else {
        Err(format!("No parser available for URL: {}", url))
    }
}
