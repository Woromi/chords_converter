use deunicode::deunicode;
use heck::ToUpperCamelCase;
use reqwest::Error;
use std::env;
use std::fs::{self, File};
use std::path::PathBuf;

mod models;
mod parsers;
mod printers;

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

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: cargo run -- <URL> [OUTPUT_FORMAT]");
        eprintln!("Output Formats: text, latex, debug (default: text)");
        std::process::exit(1);
    }

    let target_url = &args[1];
    let format_arg = if args.len() > 2 { &args[2] } else { "text" };

    let parser = match parsers::get_parser(target_url) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parser Error: {}", e);
            match fetch_html(target_url).await {
                Ok(html) => {
                    print!("{}", html);
                }
                Err(_e) => {}
            }
            std::process::exit(1);
        }
    };

    let printer = match printers::get_printer(format_arg) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Printer Error: {}", e);
            std::process::exit(1);
        }
    };

    match fetch_html(target_url).await {
        Ok(html) => {
            let song = parser.parse(target_url, &html);

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
                    // Because File implements Write, we can pass &mut file directly!
                    if let Err(e) = printer.print(&song, &mut file) {
                        eprintln!("Failed to write to file: {}", e);
                    } else {
                        println!("Successfully saved!");
                    }
                }
                Err(e) => eprintln!("Failed to create file: {}", e),
            }
        }
        Err(e) => {
            eprintln!("Network Error while fetching the page: {}", e);
            std::process::exit(1);
        }
    }
}
