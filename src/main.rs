use deunicode::deunicode;
use heck::ToUpperCamelCase;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, UPGRADE_INSECURE_REQUESTS,
};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod models;
mod parsers;
mod printers;
mod tex;

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
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
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
fn parse_file(path: &str, content: &str) -> Result<Song, String> {
    if !looks_like_html(content) {
        return Ok(parsers::text::TextParser.parse(path, content));
    }

    // A page saved while the site was showing its bot check holds no song;
    // read as one, it would come out as an empty song by "Unknown Artist".
    if looks_like_a_bot_check(content) {
        return Err(format!(
            "'{}' is a bot check the site showed instead of the song, not the song \
             itself. Save the page again once the song is on screen.",
            path
        ));
    }

    match parsers::sniff_parser(content) {
        Some(parser) => {
            // A saved page has no address of its own; the one it records is
            // worth more to the songbook than the path it was saved to.
            let source = parsers::source_url(content).unwrap_or_else(|| path.to_string());
            Ok(parser.parse(&source, content))
        }
        None => {
            eprintln!(
                "Warning: '{}' is a web page from a site no parser knows. \
                 Reading it as plain text.",
                path
            );
            Ok(parsers::text::TextParser.parse(path, content))
        }
    }
}

/// Tells a saved page from a song written out as text.
fn looks_like_html(content: &str) -> bool {
    let head: String = content
        .chars()
        .take(2000)
        .collect::<String>()
        .to_lowercase();
    head.contains("<!doctype html") || head.contains("<html")
}

/// Reads a pasted song from standard input until EOF (Ctrl-D).
fn read_stdin() -> std::io::Result<String> {
    eprintln!("Paste your chords, then press Ctrl-D (EOF) to convert:");
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;
    Ok(buffer)
}

/// Reads the song from wherever the source names: pasted text, a website or
/// a local file.
async fn acquire_song(input: &str) -> Result<Song, String> {
    if input == "-" || input == "--paste" {
        let text = read_stdin().map_err(|e| format!("Failed to read from stdin: {}", e))?;
        return Ok(parsers::text::TextParser.parse("(pasted)", &text));
    }
    if is_url(input) {
        let parser = parsers::get_parser(input).map_err(|e| format!("Parser Error: {}", e))?;
        let html = fetch_html(input).await?;
        return Ok(parser.parse(input, &html));
    }
    if !Path::new(input).is_file() {
        return Err(format!(
            "Input Error: '{}' is not a URL, not '-', and not an existing file.",
            input
        ));
    }
    let text =
        fs::read_to_string(input).map_err(|e| format!("Failed to read file '{}': {}", input, e))?;
    parse_file(input, &text)
}

/// Where the converted song goes.
#[derive(Debug, PartialEq)]
enum Output {
    /// `out/<format>/<Artist>/<Title>.<ext>`, named after the song.
    Default,
    /// Standard output, for a caller that keeps the song itself.
    Stdout,
    File(PathBuf),
}

/// The default place a song is written to: out/<format>/<artist>/<title>.<ext>.
fn default_path(song: &Song, format: &str, printer: &dyn SongPrinter) -> PathBuf {
    let mut path = PathBuf::from("out");
    path.push(format.to_lowercase());
    path.push(sanitize_filename(&song.artist));
    path.push(format!(
        "{}.{}",
        sanitize_filename(&song.title),
        printer.extension()
    ));
    path
}

/// Writes the song where it was asked to go, tidied first when asked to be.
/// The two lines printed on the way are what a caller reads the path from
/// (ZubuvZpevnikUi does), so they keep their wording.
fn save_song(
    song: &Song,
    format: &str,
    printer: &dyn SongPrinter,
    output: &Output,
    tidy: bool,
) -> Result<(), String> {
    // The song is printed in full before anything is written, so a failure
    // halfway leaves no half-written file behind.
    let mut bytes = Vec::new();
    printer
        .print(song, &mut bytes)
        .map_err(|e| format!("Failed to write to file: {}", e))?;
    let mut text = String::from_utf8(bytes).map_err(|e| format!("Failed to write: {}", e))?;
    if tidy {
        // The same tidying `tidy` does to a song in the songbook, done to the
        // fresh draft's text.
        let (tidied, changes) = tex::tidy::tidy(&text);
        report_changes("draft", &changes);
        text = tidied;
    }

    let path = match output {
        Output::Stdout => return write_stdout(&text),
        Output::File(path) => path.clone(),
        Output::Default => default_path(song, format, printer),
    };

    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|e| format!("Failed to create directories: {}", e))?;
    }
    println!("Saving output to: {}", path.display());
    fs::write(&path, text).map_err(|e| format!("Failed to create file: {}", e))?;
    println!("Successfully saved!");
    Ok(())
}

/// Prints text to standard output. A reader that stops early (`| head`) has
/// all it wanted.
fn write_stdout(text: &str) -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|_| stdout.flush())
    {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
            Err(format!("Failed to write the song: {}", e))
        }
        _ => Ok(()),
    }
}

/// Tells the user, on stderr, what tidying changed and where.
fn report_changes(name: &str, changes: &[tex::tidy::Change]) {
    for change in changes {
        eprintln!("{}:{}: {}", name, change.line, change.description);
    }
}

/// Tidies a song file. By default the file itself is rewritten, but only
/// when something changed, and through a file beside it that then takes its
/// place, so the song is never left half written.
fn tidy_file(file: &Path, output: &Output) -> Result<(), String> {
    let text = fs::read_to_string(file)
        .map_err(|e| format!("Failed to read file '{}': {}", file.display(), e))?;
    let (tidied, changes) = tex::tidy::tidy(&text);
    report_changes(&file.display().to_string(), &changes);

    write_back(file, &tidied, !changes.is_empty(), output)
}

/// Writes a changed song where it was asked to go: standard output, another
/// file, or back over the song itself. The song itself is rewritten only
/// when something changed, and through a file beside it that then takes its
/// place, so it is never left half written.
fn write_back(file: &Path, text: &str, changed: bool, output: &Output) -> Result<(), String> {
    match output {
        Output::Stdout => write_stdout(text),
        Output::File(path) => fs::write(path, text)
            .map_err(|e| format!("Failed to write '{}': {}", path.display(), e)),
        Output::Default if !changed => Ok(()),
        Output::Default => {
            let mut temporary = file.as_os_str().to_owned();
            temporary.push(".tidy");
            let temporary = PathBuf::from(temporary);
            fs::write(&temporary, text)
                .and_then(|_| fs::rename(&temporary, file))
                .map_err(|e| {
                    let _ = fs::remove_file(&temporary);
                    format!("Failed to write '{}': {}", file.display(), e)
                })
        }
    }
}

/// The pages a song may take on a paper: one, or two facing A5 pages for a
/// song marked `pap={…a5x2}`, which is meant to open across a spread.
fn pages_allowed(text: &str, paper: &str) -> u32 {
    let marked_spread = text
        .lines()
        .map(tex::code_of)
        .filter_map(|code| code.find("pap={").map(|at| code[at..].to_string()))
        .any(|pap| {
            pap.split(['{', '}', ','])
                .any(|p| p.trim() == format!("{paper}x2"))
        });
    if marked_spread { 2 } else { 1 }
}

/// Lists the proposals for a song, and with `--measure` says whether it fits.
///
/// `--fit a4` offers the changes that act on every paper. `--fit a5` does
/// too while the song still overflows A4 (it overflows A5 then as well), and
/// the changes for small paper only once A4 fits. Without `--measure` there
/// is no telling, so both stages are listed, every-paper changes first.
fn suggest(
    file: &Path,
    fit: Option<&str>,
    measure: bool,
    songbook: Option<&Path>,
    json: bool,
) -> Result<(), String> {
    use tex::fit::Stage;
    let text = fs::read_to_string(file)
        .map_err(|e| format!("Failed to read file '{}': {}", file.display(), e))?;
    let all = tex::fit::proposals(&text);

    let default_songbook = PathBuf::from("../ZubuvZpevnik");
    let songbook = songbook.unwrap_or(&default_songbook);
    let mut measured: BTreeMap<String, tex::measure::Measured> = BTreeMap::new();
    let mut measure_on = |paper: &str| -> Result<tex::measure::Measured, String> {
        let m = tex::measure::measure(songbook, &text, &[paper])?
            .remove(paper)
            .ok_or_else(|| format!("No measurement for {}", paper))?;
        measured.insert(paper.to_string(), m.clone());
        Ok(m)
    };

    let mut stages = vec![Stage::AllPapers, Stage::SmallPaper];
    let mut fits = None;
    let mut note = String::new();
    match (fit, measure) {
        (Some("a4"), false) => stages = vec![Stage::AllPapers],
        (Some("a4"), true) => {
            stages = vec![Stage::AllPapers];
            if measure_on("a4")?.fits(1) {
                fits = Some(true);
            }
        }
        (Some("a5"), true) => {
            if !measure_on("a4")?.fits(1) {
                stages = vec![Stage::AllPapers];
                note = "It does not fit A4 yet, and A5 not either then: first the changes for every paper.".into();
            } else if measure_on("a5")?.fits(pages_allowed(&text, "a5")) {
                fits = Some(true);
            } else {
                stages = vec![Stage::SmallPaper];
                note = "It fits A4: the changes for small paper only.".into();
            }
        }
        (Some("a5"), false) | (None, _) => {
            if measure {
                measure_on("a4")?;
                measure_on("a5")?;
            }
        }
        (Some(other), _) => return Err(format!("Unknown paper '{}': a4 or a5.", other)),
    }
    if fits == Some(true) {
        stages.clear();
    } else if fit.is_some() && measure {
        fits = Some(false);
    }
    let shown: Vec<&tex::fit::Proposal> =
        all.iter().filter(|p| stages.contains(&p.stage)).collect();

    if json {
        let proposals: Vec<serde_json::Value> = shown
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "kind": p.kind,
                    "stage": p.stage as u8,
                    "line": p.line,
                    "description": p.description,
                    "gain_lines": p.gain_lines,
                    "edits": p.edits.iter().map(|e| serde_json::json!({
                        "start": e.span.start,
                        "end": e.span.end,
                        "replacement": e.replacement,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let measured: serde_json::Map<String, serde_json::Value> = measured
            .iter()
            .map(|(paper, m)| {
                (
                    paper.clone(),
                    serde_json::json!({
                        "pages": m.pages,
                        "wraps": m.wraps.iter().map(|w| serde_json::json!({
                            "line": w.line,
                            "in_chorus": w.in_chorus,
                        })).collect::<Vec<_>>(),
                        "fits": m.fits(pages_allowed(&text, paper)),
                    }),
                )
            })
            .collect();
        let out = serde_json::json!({
            "file": file.display().to_string(),
            "fit": fit,
            "fits": fits,
            "measured": measured,
            "note": note,
            "proposals": proposals,
        });
        return write_stdout(&format!("{}\n", out));
    }

    let mut out = format!("{}\n", file.display());
    for (paper, m) in &measured {
        let allowed = pages_allowed(&text, paper);
        let pages = if m.pages == 1 {
            "1 page".to_string()
        } else {
            format!("{} pages", m.pages)
        };
        let verdict = if m.fits(allowed) {
            "fits"
        } else {
            "does not fit"
        };
        out.push_str(&format!(
            "  {}: {}, {}",
            paper.to_uppercase(),
            pages,
            verdict
        ));
        if !m.wraps.is_empty() {
            let lines: Vec<String> = m
                .wraps
                .iter()
                .map(|w| match w.line {
                    Some(line) => format!("line {}", line),
                    None => "a line of a chorus".to_string(),
                })
                .collect();
            out.push_str(&format!(
                "; too wide: {} (re-break by hand)",
                lines.join(", ")
            ));
        }
        out.push('\n');
    }
    if !note.is_empty() {
        out.push_str(&format!("  {}\n", note));
    }
    if fits == Some(true) {
        let paper = fit.unwrap_or("a5");
        out.push_str(&format!(
            "  It fits {}. Record it in the songbook with:\n    scripts/checkpaper.py --measure {} --write <song>\n",
            paper.to_uppercase(),
            paper
        ));
    } else if shown.is_empty() {
        out.push_str("  Nothing left to propose.\n");
    } else {
        out.push_str("  Proposals, best first:\n");
        for p in &shown {
            out.push_str(&format!(
                "    {:<28} line {:>3}  {} (~{} lines)\n",
                p.id, p.line, p.description, p.gain_lines
            ));
        }
        out.push_str(&format!(
            "  Apply one with: chords_converter apply {} --id <id>, then ask again.\n",
            file.display()
        ));
    }
    write_stdout(&out)
}

/// Applies the chosen proposals to a song. The proposals are found afresh
/// in the file as it is now, so an id from an older `suggest` that no longer
/// applies is refused rather than applied to the wrong bytes.
fn apply_file(file: &Path, ids: &[String], output: &Output) -> Result<(), String> {
    let text = fs::read_to_string(file)
        .map_err(|e| format!("Failed to read file '{}': {}", file.display(), e))?;
    let all = tex::fit::proposals(&text);
    let mut chosen = Vec::new();
    for id in ids {
        let proposal = all.iter().find(|p| &p.id == id).ok_or_else(|| {
            format!(
                "No proposal '{}' for '{}' (ask again with suggest).",
                id,
                file.display()
            )
        })?;
        chosen.push(proposal);
    }
    let changed = tex::fit::apply(&text, &chosen)?;
    for p in &chosen {
        eprintln!("{}:{}: {}", file.display(), p.line, p.description);
    }
    write_back(file, &changed, true, output)
}

const USAGE: &str = "\
Usage:
  chords_converter <SOURCE> [FORMAT]
  chords_converter convert <SOURCE> [--format FORMAT] [--tidy] [-o FILE | -o -]
  chords_converter tidy <SONG.tex> [-o FILE | -o -]
  chords_converter suggest <SONG.tex> [--fit a4|a5] [--measure] [--songbook DIR] [--json]
  chords_converter apply <SONG.tex> --id ID [--id ID ...] [-o FILE | -o -]

SOURCE   a URL of a supported website, a local text file (chords over
         lyrics), a page saved from a browser, or - to paste the song
FORMAT   latex (default), text, debug

Without -o the song is written to out/<format>/<Artist>/<Title>.<ext>
and the path is printed as 'Saving output to: <path>'. -o - prints the
song to standard output instead.

tidy improves a LaTeX song without changing anything else in it: a chorus
typed out again is replaced by \\insertchorus, and a chord drawn inside its
syllable moves to the syllable's start. It rewrites the file unless
-o is given, and lists every change on stderr. convert --tidy does the same
to the draft.

suggest lists changes to what a song prints, best first, for a person to
accept or reject: chords hidden where they repeat ones already shown, a
chorus shortened to its reminder line, a smaller type on small paper.
--fit a4|a5 keeps to the ones that help fit that paper; --measure typesets
the song with the songbook's own scripts/checkpaper.py (the songbook is
../ZubuvZpevnik unless --songbook says otherwise) and says whether it fits.
apply makes the chosen ones; apply one, then ask again.";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    Help,
    Convert {
        source: String,
        format: String,
        output: Output,
        tidy: bool,
    },
    Tidy {
        file: PathBuf,
        output: Output,
    },
    Suggest {
        file: PathBuf,
        fit: Option<String>,
        measure: bool,
        songbook: Option<PathBuf>,
        json: bool,
    },
    Apply {
        file: PathBuf,
        ids: Vec<String>,
        output: Output,
    },
}

/// Reads the command line. The first form, `<SOURCE> [FORMAT]`, is the one
/// the tool always had, and callers rely on it, so it keeps working as it
/// was; `convert` is the same with options.
fn parse_args(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Err("No source given.".to_string());
    };
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "convert" => parse_convert(&args[1..]),
        "tidy" => parse_tidy(&args[1..]),
        "suggest" => parse_suggest(&args[1..]),
        "apply" => parse_apply(&args[1..]),
        _ => match args {
            [source] => Ok(Command::Convert {
                source: source.clone(),
                format: "latex".to_string(),
                output: Output::Default,
                tidy: false,
            }),
            [source, format] => Ok(Command::Convert {
                source: source.clone(),
                format: format.clone(),
                output: Output::Default,
                tidy: false,
            }),
            _ => Err(format!("Unexpected argument '{}'.", args[2])),
        },
    }
}

fn parse_convert(args: &[String]) -> Result<Command, String> {
    let mut source = None;
    let mut format = "latex".to_string();
    let mut output = Output::Default;
    let mut tidy = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{} needs a value.", name))
        };
        match arg.as_str() {
            "--format" | "-f" => format = value(arg)?,
            "--tidy" => tidy = true,
            "--output" | "-o" => {
                output = match value(arg)?.as_str() {
                    "-" => Output::Stdout,
                    path => Output::File(PathBuf::from(path)),
                }
            }
            // A lone "-" is the pasted source, not an option.
            option if option.starts_with('-') && option != "-" => {
                return Err(format!("Unknown option '{}'.", option));
            }
            _ if source.is_some() => return Err(format!("Unexpected argument '{}'.", arg)),
            _ => source = Some(arg.clone()),
        }
    }
    let source = source.ok_or_else(|| "convert needs a SOURCE.".to_string())?;
    if tidy && !format.eq_ignore_ascii_case("latex") {
        return Err("--tidy works on LaTeX output only.".to_string());
    }
    Ok(Command::Convert {
        source,
        format,
        output,
        tidy,
    })
}

fn parse_suggest(args: &[String]) -> Result<Command, String> {
    let mut file = None;
    let mut fit = None;
    let mut measure = false;
    let mut songbook = None;
    let mut json = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{} needs a value.", arg))
        };
        match arg.as_str() {
            "--fit" => {
                let paper = value()?.to_lowercase();
                if paper != "a4" && paper != "a5" {
                    return Err(format!("--fit takes a4 or a5, not '{}'.", paper));
                }
                fit = Some(paper);
            }
            "--measure" => measure = true,
            "--songbook" => songbook = Some(PathBuf::from(value()?)),
            "--json" => json = true,
            option if option.starts_with('-') => {
                return Err(format!("Unknown option '{}'.", option));
            }
            _ if file.is_some() => return Err(format!("Unexpected argument '{}'.", arg)),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    let file = file.ok_or_else(|| "suggest needs a song file.".to_string())?;
    Ok(Command::Suggest {
        file,
        fit,
        measure,
        songbook,
        json,
    })
}

fn parse_apply(args: &[String]) -> Result<Command, String> {
    let mut file = None;
    let mut ids = Vec::new();
    let mut output = Output::Default;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{} needs a value.", arg))
        };
        match arg.as_str() {
            "--id" => ids.push(value()?),
            "--output" | "-o" => {
                output = match value()?.as_str() {
                    "-" => Output::Stdout,
                    path => Output::File(PathBuf::from(path)),
                }
            }
            option if option.starts_with('-') => {
                return Err(format!("Unknown option '{}'.", option));
            }
            _ if file.is_some() => return Err(format!("Unexpected argument '{}'.", arg)),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    let file = file.ok_or_else(|| "apply needs a song file.".to_string())?;
    if ids.is_empty() {
        return Err("apply needs at least one --id.".to_string());
    }
    Ok(Command::Apply { file, ids, output })
}

fn parse_tidy(args: &[String]) -> Result<Command, String> {
    let mut file = None;
    let mut output = Output::Default;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" | "-o" => {
                output = match args.next().map(String::as_str) {
                    Some("-") => Output::Stdout,
                    Some(path) => Output::File(PathBuf::from(path)),
                    None => return Err(format!("{} needs a value.", arg)),
                }
            }
            option if option.starts_with('-') => {
                return Err(format!("Unknown option '{}'.", option));
            }
            _ if file.is_some() => return Err(format!("Unexpected argument '{}'.", arg)),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    let file = file.ok_or_else(|| "tidy needs a song file.".to_string())?;
    Ok(Command::Tidy { file, output })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let command = match parse_args(&args) {
        Ok(command) => command,
        Err(e) => {
            eprintln!("{}\n\n{}", e, USAGE);
            std::process::exit(2);
        }
    };

    let result = match command {
        Command::Help => {
            println!("{}", USAGE);
            return;
        }
        Command::Tidy { file, output } => tidy_file(&file, &output),
        Command::Suggest {
            file,
            fit,
            measure,
            songbook,
            json,
        } => suggest(&file, fit.as_deref(), measure, songbook.as_deref(), json),
        Command::Apply { file, ids, output } => apply_file(&file, &ids, &output),
        Command::Convert {
            source,
            format,
            output,
            tidy,
        } => convert(&source, &format, &output, tidy).await,
    };

    if let Err(e) = result {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

async fn convert(source: &str, format: &str, output: &Output, tidy: bool) -> Result<(), String> {
    let printer = printers::get_printer(format).map_err(|e| format!("Printer Error: {}", e))?;
    let song = acquire_song(source).await?;
    save_song(&song, format, printer.as_ref(), output, tidy)
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

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn convert(source: &str, format: &str, output: Output) -> Command {
        Command::Convert {
            source: source.to_string(),
            format: format.to_string(),
            output,
            tidy: false,
        }
    }

    /// ZubuvZpevnikUi runs `chords_converter <source> latex`; that form has to
    /// mean what it always meant.
    #[test]
    fn the_original_form_still_works() {
        assert_eq!(
            parse_args(&args(&["https://a.cz/song", "latex"])),
            Ok(convert("https://a.cz/song", "latex", Output::Default))
        );
        assert_eq!(
            parse_args(&args(&["song.txt"])),
            Ok(convert("song.txt", "latex", Output::Default))
        );
        assert!(parse_args(&args(&["a", "latex", "extra"])).is_err());
    }

    #[test]
    fn convert_takes_options_in_any_order() {
        assert_eq!(
            parse_args(&args(&[
                "convert",
                "-o",
                "-",
                "page.html",
                "--format",
                "text"
            ])),
            Ok(convert("page.html", "text", Output::Stdout))
        );
        assert_eq!(
            parse_args(&args(&["convert", "page.html", "-o", "x/Song.tex"])),
            Ok(convert(
                "page.html",
                "latex",
                Output::File("x/Song.tex".into())
            ))
        );
        // A lone dash is the pasted song.
        assert_eq!(
            parse_args(&args(&["convert", "-"])),
            Ok(convert("-", "latex", Output::Default))
        );
    }

    #[test]
    fn convert_refuses_what_it_does_not_know() {
        assert!(parse_args(&args(&["convert"])).is_err());
        assert!(parse_args(&args(&["convert", "a", "b"])).is_err());
        assert!(parse_args(&args(&["convert", "a", "--bogus"])).is_err());
        assert!(parse_args(&args(&["convert", "a", "-o"])).is_err());
        assert!(parse_args(&args(&[])).is_err());
    }

    #[test]
    fn a_saved_bot_check_is_not_read_as_a_song() {
        let error = parse_file("page.html", CHALLENGE).unwrap_err();
        assert!(error.contains("bot check"), "{error}");
    }

    #[test]
    fn tidy_takes_a_file_and_an_output() {
        assert_eq!(
            parse_args(&args(&["tidy", "Song.tex"])),
            Ok(Command::Tidy {
                file: "Song.tex".into(),
                output: Output::Default
            })
        );
        assert_eq!(
            parse_args(&args(&["tidy", "-o", "-", "Song.tex"])),
            Ok(Command::Tidy {
                file: "Song.tex".into(),
                output: Output::Stdout
            })
        );
        assert!(parse_args(&args(&["tidy"])).is_err());
        assert!(parse_args(&args(&["tidy", "a.tex", "b.tex"])).is_err());
    }

    #[test]
    fn convert_tidies_latex_only() {
        assert_eq!(
            parse_args(&args(&["convert", "page.html", "--tidy"])),
            Ok(Command::Convert {
                source: "page.html".to_string(),
                format: "latex".to_string(),
                output: Output::Default,
                tidy: true,
            })
        );
        assert!(parse_args(&args(&["convert", "page.html", "--tidy", "-f", "text"])).is_err());
    }

    #[test]
    fn suggest_and_apply_read_their_options() {
        assert_eq!(
            parse_args(&args(&[
                "suggest",
                "S.tex",
                "--fit",
                "A5",
                "--measure",
                "--json"
            ])),
            Ok(Command::Suggest {
                file: "S.tex".into(),
                fit: Some("a5".to_string()),
                measure: true,
                songbook: None,
                json: true,
            })
        );
        assert!(parse_args(&args(&["suggest", "S.tex", "--fit", "letter"])).is_err());
        assert!(parse_args(&args(&["suggest"])).is_err());

        assert_eq!(
            parse_args(&args(&[
                "apply",
                "S.tex",
                "--id",
                "chordsoff@10",
                "--id",
                "x@2",
                "-o",
                "-"
            ])),
            Ok(Command::Apply {
                file: "S.tex".into(),
                ids: vec!["chordsoff@10".to_string(), "x@2".to_string()],
                output: Output::Stdout,
            })
        );
        assert!(parse_args(&args(&["apply", "S.tex"])).is_err());
    }

    #[test]
    fn a_song_marked_for_a_spread_may_take_two_a5_pages() {
        assert_eq!(pages_allowed("    pap={a4,a5x2},\n", "a5"), 2);
        assert_eq!(pages_allowed("    pap={a4,a5},\n", "a5"), 1);
        assert_eq!(pages_allowed("    % pap={a5x2},\n", "a5"), 1);
        assert_eq!(pages_allowed("    pap={a4,a5x2},\n", "a4"), 1);
    }
}
