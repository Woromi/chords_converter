use crate::models::Song;
use scraper::{Html, Selector};

pub mod blivnik_cz;
pub mod kytary_cz;
pub mod pisnicky_akody_cz;
pub mod text;
pub mod ultimate_guitar_com;

pub trait SongParser {
    // We now pass the exact link into the parse method
    fn parse(&self, url: &str, html_content: &str) -> Song;
}

pub fn get_parser(url: &str) -> Result<Box<dyn SongParser>, String> {
    if url.contains("akordy.kytary.cz") {
        Ok(Box::new(kytary_cz::KytaryCzParser))
    } else if url.contains("pisnicky-akordy.cz") {
        Ok(Box::new(pisnicky_akody_cz::PisnickyAkordyCzParser))
    } else if url.contains("blivnik.cz") {
        Ok(Box::new(blivnik_cz::BlivnikCzParser))
    } else if url.contains("ultimate-guitar.com") {
        Ok(Box::new(ultimate_guitar_com::UltimateGuitarComParser))
    } else {
        Err(format!("No parser available for URL: {}", url))
    }
}

/// Recognises a page saved from a browser by the markup its site is built on.
/// A saved page carries no address of its own — and blivnik.cz does not even
/// write one into the file — so the landmark each parser reads the song out of
/// is what identifies it.
pub fn sniff_parser(html_content: &str) -> Option<Box<dyn SongParser>> {
    if html_content.contains("js-store") && html_content.contains("data-content") {
        Some(Box::new(ultimate_guitar_com::UltimateGuitarComParser))
    } else if html_content.contains("smartchordsheet") || html_content.contains("scs-section") {
        Some(Box::new(kytary_cz::KytaryCzParser))
    } else if html_content.contains("origText") {
        Some(Box::new(blivnik_cz::BlivnikCzParser))
    } else if html_content.contains("songtext") && html_content.contains("aline") {
        Some(Box::new(pisnicky_akody_cz::PisnickyAkordyCzParser))
    } else {
        None
    }
}

/// The address a saved page was served from, as the page itself records it.
/// It is what the song should name as its source — the path it was saved to
/// says nothing to whoever reads the songbook later.
pub fn source_url(html_content: &str) -> Option<String> {
    let document = Html::parse_document(html_content);
    let canonical = Selector::parse("link[rel=canonical]").ok()?;
    let og_url = Selector::parse(r#"meta[property="og:url"]"#).ok()?;

    let href = document
        .select(&canonical)
        .next()
        .and_then(|el| el.value().attr("href"));
    let content = document
        .select(&og_url)
        .next()
        .and_then(|el| el.value().attr("content"));

    href.or(content)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}
