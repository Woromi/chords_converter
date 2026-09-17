use deunicode::deunicode;
use heck::ToUpperCamelCase;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, UPGRADE_INSECURE_REQUESTS,
};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// A current desktop browser. Several of the supported sites — ultimate-guitar
/// .com above all — answer a request that does not look like one with a bot
/// check instead of the song.
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/124.0.0.0 Safari/537.36";

/// How often a request is retried when the site turns it away. The blocking is
/// usually short-lived, so a couple of attempts with a growing pause are enough.
const ATTEMPTS: u32 = 3;

/// Builds a client that presents itself as a browser: the headers one sends,
/// a cookie jar to carry whatever the site sets on the way in, and transparent
/// gzip so the `Accept-Encoding` a browser sends can be honoured.
fn build_client() -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.9,cs;q=0.8"),
    );
    headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));

    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .cookie_store(true)
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .build()
}

/// Fetches a page, retrying the refusals that are worth retrying.
async fn fetch_html(url: &str) -> Result<String, String> {
    let client = build_client().map_err(|e| format!("Could not set the HTTP client up: {}", e))?;

    let mut last: String = String::new();
    for attempt in 1..=ATTEMPTS {
        if attempt > 1 {
            // Back off a little before asking again: 2s, then 4s.
            tokio::time::sleep(Duration::from_secs(2 << (attempt - 2))).await;
            eprintln!("Retrying ({}/{}) ...", attempt, ATTEMPTS);
        }

        let response = match client.get(url).send().await {
            Ok(response) => response,
            Err(e) => {
                last = format!("the request failed: {}", e);
                continue;
            }
        };

        let status = response.status();
        if status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| format!("Could not read the page: {}", e))?;
            if looks_like_a_bot_check(&body) {
                last = "the site answered with a bot check".to_string();
                continue;
            }
            return Ok(body);
        }

        last = format!("the site answered {}", status);
        // A refusal the site will not reconsider — asking again is pointless.
        if !(status.is_server_error()
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::FORBIDDEN)
        {
            break;
        }
    }

    Err(format!(
        "Could not fetch {} — {}.\n\
         Should it keep happening, open the page in a browser and save it, then \
         pass the saved file instead.",
        url, last
    ))
}

/// Recognises the interstitials sites put in front of a page instead of
/// refusing outright, which arrive with a perfectly fine 200.
fn looks_like_a_bot_check(body: &str) -> bool {
    const SIGNS: [&str; 4] = [
        "Just a moment...",
        "cf-browser-verification",
        "challenge-platform",
        "Enable JavaScript and cookies to continue",
    ];

    body.len() < 100_000 && SIGNS.iter().any(|sign| body.contains(sign))
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
                eprintln!("{}", e);
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
