use deunicode::deunicode;
use heck::ToUpperCamelCase;
use reqwest::Error;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

mod models;
mod parsers;
mod printers;

use models::Song;
use parsers::SongParser;
use printers::SongPrinter;

// Helper to sanitize problematic characters out of filenames
fn sanitize_filename(name: &str) -> String {
    deunicode(
        &name
            .replace('/', "_")
            .replace('\\', "_")
            .replace(':', "_")
            .replace('?', "_")
            .replace('*', "_")
            .replace('"', "_")
            .replace('<', "_")
            .replace('>', "_")
            .replace('|', "_")
            .to_upper_camel_case(),
    )
}

async fn fetch_html(url: &str) -> Result<String, Error> {
    let response = reqwest::get(url).await?;
    let body = response.text().await?;
    Ok(body)
}

fn is_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
}

/// Reads a pasted song from standard input until EOF (Ctrl-D).
fn read_stdin() -> std::io::Result<String> {
    eprintln!("Paste your chords, then press Ctrl-D (EOF) to convert:");
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
}

/// Writes the parsed song to out/<format>/<artist>/<title>.<extension>.
fn save_song(song: &Song, format_arg: &str, printer: &dyn SongPrinter) {
    // 1. Sanitize folder and filenames
    let safe_artist = sanitize_filename(&song.artist);
    let safe_title = sanitize_filename(&song.title);
    let extension = printer.extension();

    // 2. Build structured path: out/<format>/<artist>/
    let mut path = PathBuf::from("out");
    path.push(format_arg);
    path.push(&safe_artist);

    // 3. Ensure the nested directories exist
    if let Err(e) = fs::create_dir_all(&path) {
        eprintln!("Failed to create directories: {}", e);
        std::process::exit(1);
    }

    // 4. Append the actual filename: <title>.<extension>
    path.push(format!("{}.{}", safe_title, extension));

    println!("Saving output to: {}", path.display());

    // 5. Create the file and execute the printer
    match File::create(&path) {
        Ok(mut file) => {
            if let Err(e) = printer.print(song, &mut file) {
                eprintln!("Failed to write to file: {}", e);
            } else {
                println!("Successfully saved!");
            }
        }
        Err(e) => eprintln!("Failed to create file: {}", e),
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: chords_converter <URL | FILE | -> [OUTPUT_FORMAT]");
        eprintln!("  <URL>   Fetch and parse a supported website");
        eprintln!("  <FILE>  Parse a local text file (chords over lyrics)");
        eprintln!("  -       Read chords from standard input (paste, then Ctrl-D)");
        eprintln!("Output Formats: text, latex, debug (default: text)");
        std::process::exit(1);
    }

    let input_arg = &args[1];
    let format_arg = if args.len() > 2 { args[2].as_str() } else { "text" };

    let printer = match printers::get_printer(format_arg) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Printer Error: {}", e);
            std::process::exit(1);
        }
    };

    // Acquire the song from the requested source: pasted text, a website, or a file.
    let song: Song = if input_arg == "-" || input_arg == "--paste" {
        match read_stdin() {
            Ok(text) => parsers::text::TextParser.parse("(pasted)", &text),
            Err(e) => {
                eprintln!("Failed to read from stdin: {}", e);
                std::process::exit(1);
            }
        }
    } else if is_url(input_arg) {
        let parser = match parsers::get_parser(input_arg) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Parser Error: {}", e);
                if let Ok(html) = fetch_html(input_arg).await {
                    print!("{}", html);
                }
                std::process::exit(1);
            }
        };
        match fetch_html(input_arg).await {
            Ok(html) => parser.parse(input_arg, &html),
            Err(e) => {
                eprintln!("Network Error while fetching the page: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Treat the argument as a local text file.
        if !Path::new(input_arg).is_file() {
            eprintln!(
                "Input Error: '{}' is not a URL, not '-', and not an existing file.",
                input_arg
            );
            std::process::exit(1);
        }
        match fs::read_to_string(input_arg) {
            Ok(text) => parsers::text::TextParser.parse(input_arg, &text),
            Err(e) => {
                eprintln!("Failed to read file '{}': {}", input_arg, e);
                std::process::exit(1);
            }
        }
    };

    save_song(&song, format_arg, printer.as_ref());
}
