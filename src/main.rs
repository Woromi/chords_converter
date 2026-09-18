use deunicode::deunicode;
use heck::ToUpperCamelCase;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, UPGRADE_INSECURE_REQUESTS,
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

/// The cookies a browser carries to a site that has already let it in, and the
/// browser they were earned by. pisnicky-akordy.cz sits behind a Cloudflare
/// challenge that is solved by running its JavaScript — which no plain HTTP
/// client does, this one included: neither the headers a browser sends nor its
/// TLS fingerprint gets past it, only the `cf_clearance` cookie the solved
/// challenge leaves behind. So the way through is to lend this tool the
/// clearance a browser already holds; see the README for where to copy it from.
///
/// Cloudflare ties that cookie to the browser that solved the challenge, so the
/// user agent has to be set to the same browser's — hence the second variable.
const COOKIE_VAR: &str = "CHORDS_COOKIE";
const USER_AGENT_VAR: &str = "CHORDS_USER_AGENT";

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

    // The clearance a browser lent us, when it did. It goes in as a default
    // header rather than into the jar: the jar only carries what the site sets
    // during this run, and a challenged site sets nothing worth keeping.
    if let Some(cookie) = env::var(COOKIE_VAR).ok().filter(|c| !c.trim().is_empty()) {
        match HeaderValue::from_str(cookie.trim()) {
            Ok(value) => {
                headers.insert(COOKIE, value);
            }
            Err(_) => eprintln!(
                "Warning: {} is not a usable Cookie header; ignoring it.",
                COOKIE_VAR
            ),
        }
    }

    // The browser the clearance was earned by, when it is not the one below.
    let user_agent = env::var(USER_AGENT_VAR)
        .ok()
        .filter(|agent| !agent.trim().is_empty())
        .unwrap_or_else(|| USER_AGENT.to_string());

    reqwest::Client::builder()
        .user_agent(user_agent)
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
        // The body is worth reading whatever the status says: a challenge is
        // served under a 403 as readily as under a 200, and reading it is the
        // only way to tell a bot check from a site that is merely refusing.
        let body = response
            .text()
            .await
            .map_err(|e| format!("Could not read the page: {}", e))?;

        if looks_like_a_bot_check(&body) {
            // Asking again would only fetch the same challenge: it is answered
            // by running its JavaScript, and a second identical request does
            // not run any. So this one is reported rather than retried.
            return Err(bot_check_message(url));
        }

        if status.is_success() {
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

/// Says what stood in the page's place and what gets past it.
///
/// Both ways out end at the same browser, because only a browser answers the
/// challenge: either it saves the page it was shown, or it lends its clearance
/// to this tool.
fn bot_check_message(url: &str) -> String {
    format!(
        "Could not fetch {url} — the site answered with a bot check rather than \
         the song.\n\
         It is solved by running the challenge's JavaScript, which this tool \
         does not do, so there are two ways round it:\n\
         \n\
         1. Open the page in a browser, save it (Ctrl-S), and pass the saved \
         file instead:\n\
         \x20      chords_converter page.html latex\n\
         \n\
         2. Lend this tool the clearance your browser already holds. In the \
         browser, on that site: DevTools → Application (Storage) → Cookies, and \
         copy the `cf_clearance` value; the user agent has to be the same \
         browser's, which `navigator.userAgent` in the console prints:\n\
         \x20      export {COOKIE_VAR}='cf_clearance=<value>'\n\
         \x20      export {USER_AGENT_VAR}='<what navigator.userAgent printed>'\n\
         \n\
         The cookie is tied to that browser and address and lasts a while, not \
         forever — when it lapses, copy it again or fall back on 1."
    )
}

/// Recognises the interstitials sites put in front of a page instead of the
/// page itself. The status they come under says nothing: ultimate-guitar.com
/// serves its challenge with a perfectly fine 200, pisnicky-akordy.cz serves
/// the same challenge with a 403, so the body is what gets read.
///
/// The signs have to be the interstitial's own. Cloudflare puts its passive
/// detection script (`/cdn-cgi/challenge-platform/...`) on ordinary pages as
/// well, so the script alone says nothing about whether the song is there —
/// taking it for a bot check turned every page of a site into one.
fn looks_like_a_bot_check(body: &str) -> bool {
    const SIGNS: [&str; 6] = [
        "<title>Just a moment...",
        "cf-browser-verification",
        "cf-challenge-running",
        "id=\"challenge-running\"",
        "challenge-error-title",
        "Enable JavaScript and cookies to continue",
    ];

    body.len() < 100_000 && SIGNS.iter().any(|sign| body.contains(sign))
}

fn is_url(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
}

/// Parses a local file: a page saved from a browser when it is one, the plain
/// "chords over lyrics" layout otherwise.
fn parse_file(path: &str, content: &str) -> Song {
    if !looks_like_html(content) {
        return parsers::text::TextParser.parse(path, content);
    }

    match parsers::sniff_parser(content) {
        Some(parser) => {
            // A saved page has no address of its own; the one it records is
            // worth more to the songbook than the path it was saved to.
            let source = parsers::source_url(content).unwrap_or_else(|| path.to_string());
            parser.parse(&source, content)
        }
        None => {
            eprintln!(
                "Warning: '{}' is a web page from a site no parser knows. \
                 Reading it as plain text.",
                path
            );
            parsers::text::TextParser.parse(path, content)
        }
    }
}

/// Tells a saved page from a song written out as text.
fn looks_like_html(content: &str) -> bool {
    let head: String = content.chars().take(2000).collect::<String>().to_lowercase();
    head.contains("<!doctype html") || head.contains("<html")
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
        eprintln!("  <FILE>  Parse a local text file (chords over lyrics) or a saved page");
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
            Ok(text) => parse_file(input_arg, &text),
            Err(e) => {
                eprintln!("Failed to read file '{}': {}", input_arg, e);
                std::process::exit(1);
            }
        }
    };

    save_song(&song, format_arg, printer.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The challenge Cloudflare stands in front of pisnicky-akordy.cz, cut
    /// down to the markup it is recognised by. The site serves it with a 403,
    /// ultimate-guitar.com serves its own with a 200 — which is why the status
    /// is not what the page is judged on.
    const CHALLENGE: &str = r#"<!DOCTYPE html><html lang="en-US"><head>
        <title>Just a moment...</title><meta name="robots" content="noindex,nofollow">
        </head><body><div class="main-wrapper" role="main"><noscript>
        <span id="challenge-error-text">Enable JavaScript and cookies to continue</span>
        </noscript></div><script>window._cf_chl_opt = {cType: 'managed'};</script>
        </body></html>"#;

    #[test]
    fn the_challenge_is_recognised_however_it_is_served() {
        assert!(looks_like_a_bot_check(CHALLENGE));
    }

    /// The song pages carry Cloudflare's passive script too; taking that for a
    /// challenge would turn every page of the site into one.
    #[test]
    fn an_ordinary_page_carrying_cloudflares_script_is_not_a_bot_check() {
        let page = format!(
            "<html><head><title>A Song | A Band</title></head><body>\
             <div id=\"songtext\"><pre>{}</pre></div>\
             <script src=\"/cdn-cgi/challenge-platform/h/g/scripts/jsd/main.js\"></script>\
             </body></html>",
            "lyrics ".repeat(200)
        );

        assert!(!looks_like_a_bot_check(&page));
    }

    /// Both ways past the challenge have to be in the message, since neither
    /// is something the tool can do on its own.
    #[test]
    fn the_message_names_both_ways_past_the_challenge() {
        let message = bot_check_message("https://pisnicky-akordy.cz/a-band/a-song");

        assert!(message.contains("https://pisnicky-akordy.cz/a-band/a-song"));
        assert!(message.contains("save it"));
        assert!(message.contains(COOKIE_VAR));
        assert!(message.contains(USER_AGENT_VAR));
    }
}
