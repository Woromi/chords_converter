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

/// How long one request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long fetching may take in all, pauses and retries included.
/// ZubuvZpevnikar stops the converter after 60 s, and a message saying the
/// site would not answer is worth more than a last try nobody hears of.
const FETCH_BUDGET: Duration = Duration::from_secs(50);

/// The least of the budget a try is worth making with.
const LEAST_WORTH: Duration = Duration::from_secs(5);

/// Whether to make attempt `attempt` with `elapsed` of the budget gone: the
/// pause before it (none, 2 s, then 4 s) and how long the request may take,
/// cut to what is left. `None` once the attempts are used up or too little
/// is left for a try worth making.
fn next_try(attempt: u32, elapsed: Duration) -> Option<(Duration, Duration)> {
    if attempt == 0 || attempt > ATTEMPTS {
        return None;
    }
    let pause = match attempt {
        1 => Duration::ZERO,
        n => Duration::from_secs(2 << (n - 2)),
    };
    let left = FETCH_BUDGET.checked_sub(elapsed + pause)?;
    (left >= LEAST_WORTH).then_some((pause, left.min(REQUEST_TIMEOUT)))
}

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
        .timeout(REQUEST_TIMEOUT)
        .build()
}

/// Fetches a page, retrying the refusals that are worth retrying. The
/// wording of its errors is read by ZubuvZpevnikar, which tells the kinds
/// of failure apart by it (CLAUDE.md lists the phrases).
async fn fetch_html(url: &str) -> Result<String, String> {
    let client = build_client().map_err(|e| format!("Could not set the HTTP client up: {}", e))?;

    let mut last: String = String::new();
    let started = std::time::Instant::now();
    let mut attempt = 1;
    while let Some((pause, timeout)) = next_try(attempt, started.elapsed()) {
        if attempt > 1 {
            // Back off a little before asking again: 2s, then 4s.
            tokio::time::sleep(pause).await;
            eprintln!("Retrying ({}/{}) ...", attempt, ATTEMPTS);
        }
        attempt += 1;

        let response = match client.get(url).timeout(timeout).send().await {
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
/// (ZubuvZpevnikar does), so they keep their wording.
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

/// Tells the user, on stderr, what tidying changed and where, as
/// `<name>:<line>: <description>`, which ZubuvZpevnikar reads (CLAUDE.md).
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

/// Which proposals a listing gives: the three commands share the format,
/// so a program shows them alike and `apply` takes ids from any of them.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Listing {
    /// `shrink`: changes that make the song shorter, to fit a page.
    Shrink,
    /// `join`: lines joined in pairs, each checked by typesetting.
    Join,
    /// `check`: corrections, whether the song fits or not.
    Check,
}

/// Lists the proposals for a song, and with `--measure` says whether it fits.
///
/// `--fit a4` offers the changes that act on every paper. `--fit a5` offers
/// those first while the song still overflows A4 (it overflows A5 then as
/// well), the ones for small paper after them, and only those once A4 fits. Without `--measure` there
/// is no telling, so both stages are listed, every-paper changes first.
fn list_proposals(
    file: &Path,
    fit: Option<&str>,
    measure: bool,
    songbook: Option<&Path>,
    package: Option<&Path>,
    json: bool,
    listing: Listing,
) -> Result<(), String> {
    let joins = listing == Listing::Join;
    use tex::fit::Stage;
    let text = fs::read_to_string(file)
        .map_err(|e| format!("Failed to read file '{}': {}", file.display(), e))?;
    let all = tex::fit::proposals(&text);

    let default_songbook = PathBuf::from("../ZubuvZpevnik");
    let songbook = songbook.unwrap_or(&default_songbook);
    let mut measured: BTreeMap<String, tex::measure::Measured> = BTreeMap::new();
    let mut measure_on = |paper: &str| -> Result<tex::measure::Measured, String> {
        if let Some(m) = measured.get(paper) {
            return Ok(m.clone());
        }
        let m = tex::measure::measure(songbook, package, &text, &[paper])?
            .remove(paper)
            .ok_or_else(|| format!("No measurement for {}", paper))?;
        measured.insert(paper.to_string(), m.clone());
        Ok(m)
    };

    let mut stages = vec![Stage::AllPapers, Stage::SmallPaper];
    let mut fits = None;
    let mut note = String::new();
    match (fit, measure && listing == Listing::Shrink) {
        (Some("a4"), false) => stages = vec![Stage::AllPapers],
        (Some("a4"), true) => {
            stages = vec![Stage::AllPapers];
            if measure_on("a4")?.fits(1) {
                fits = Some(true);
            }
        }
        (Some("a5"), true) => {
            if !measure_on("a4")?.fits(1) {
                // Both, every paper first: once those run out, the ones for
                // small paper still help A5 (the user).
                note = "It does not fit A4 yet, and A5 not either then: first the changes for every paper, then those for small paper only.".into();
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
    // Joined lines are checked by typesetting the joined song, a run per
    // kind of part, which is too slow for every shrink: they have their
    // own command, `join`. No line they lengthen may be too wide already,
    // and none may come out too wide, on either paper.
    let mut shown: Vec<&tex::fit::Proposal> = Vec::new();
    // For `join`: each kind of part not offered, and why, so a person can
    // tell lines too wide from lines that could not be paired at all.
    let mut not_joined: Vec<String> = Vec::new();
    if joins {
        for paper in ["a4", "a5"] {
            measure_on(paper)?;
        }
        not_joined = tex::fit::unjoinable(&text);
        for p in all.iter().filter(|p| p.kind == "joinlines") {
            let kind = p.joined_kind().unwrap_or("part");
            match too_wide(
                &text,
                &mut |t: &str, papers: &[&str]| tex::measure::measure(songbook, package, t, papers),
                &measured,
                p,
            )? {
                None => shown.push(p),
                Some(why) => not_joined.push(format!("{kind}: {why}")),
            }
        }
    } else if listing == Listing::Check {
        // Corrections in the order of the song.
        shown = all.iter().filter(|p| p.is_correction()).collect();
        shown.sort_by_key(|p| p.line);
    } else {
        shown = all
            .iter()
            .filter(|p| !p.needs_width_check() && !p.is_correction() && stages.contains(&p.stage))
            .collect();
        // Changes that lengthen lines (the second voice inline) only once
        // typeset with them, so only with --measure.
        if measure {
            let lengthening: Vec<&tex::fit::Proposal> = all
                .iter()
                .filter(|p| {
                    p.needs_width_check() && p.kind != "joinlines" && stages.contains(&p.stage)
                })
                .collect();
            for paper in lengthening.iter().flat_map(|p| p.checked_papers()) {
                measure_on(paper)?;
            }
            for p in lengthening {
                if too_wide(
                    &text,
                    &mut |t: &str, papers: &[&str]| {
                        tex::measure::measure(songbook, package, t, papers)
                    },
                    &measured,
                    p,
                )?
                .is_none()
                {
                    shown.push(p);
                }
            }
            // Back in the order the proposals were ranked in.
            shown.sort_by_key(|p| all.iter().position(|q| std::ptr::eq(q, *p)));
        }
    }

    if json {
        let out = json_listing(
            file,
            &text,
            fit,
            fits,
            &measured,
            &note,
            &shown,
            &not_joined,
        );
        return write_stdout(&format!("{out}\n"));
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
    } else if shown.is_empty() && listing == Listing::Check {
        out.push_str("  Nothing to correct found.\n");
    } else if listing == Listing::Check {
        out.push_str("  Corrections, for a person to judge:\n");
        for p in &shown {
            out.push_str(&format!("    {:<28} {}\n", p.id, p.description));
        }
        out.push_str(&format!(
            "  Apply one with: chords_converter apply {} --id <id>, then run tidy.\n",
            file.display()
        ));
    } else if shown.is_empty() && joins {
        out.push_str("  No lines can be joined in pairs and still fit A4 and A5.\n");
        push_not_joined(&mut out, &not_joined);
    } else if shown.is_empty() {
        out.push_str("  Nothing left to propose.\n");
    } else {
        out.push_str("  Proposals by paper, the most readable first (score 0-100):\n");
        let mut stage = None;
        for p in &shown {
            if stage != Some(p.stage) {
                stage = Some(p.stage);
                out.push_str(match p.stage {
                    Stage::AllPapers => "   for every paper:\n",
                    Stage::SmallPaper => "   for small paper only:\n",
                });
            }
            out.push_str(&format!(
                "    {:>3} {:<11} {:<28} {} (~{} lines)\n",
                p.score,
                p.grade(),
                p.id,
                p.description,
                p.gain_lines
            ));
        }
        push_not_joined(&mut out, &not_joined);
        out.push_str(&format!(
            "  Apply one with: chords_converter apply {} --id <id>, then ask again.\n",
            file.display()
        ));
    }
    write_stdout(&out)
}

/// The listing as a program reads it, what `--json` prints. Its shape is
/// part of the interface (CLAUDE.md): ZubuvZpevnikar reads every field,
/// so a change here is a version bump, and the test
/// `the_json_listing_keeps_its_shape` pins it.
#[allow(clippy::too_many_arguments)]
fn json_listing(
    file: &Path,
    text: &str,
    fit: Option<&str>,
    fits: Option<bool>,
    measured: &BTreeMap<String, tex::measure::Measured>,
    note: &str,
    shown: &[&tex::fit::Proposal],
    not_joined: &[String],
) -> serde_json::Value {
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
                "score": p.score,
                "grade": p.grade(),
                "category": p.category(),
                "dismiss": tex::fit::dismissal(text, p).map(|e| serde_json::json!({
                    "start": e.span.start,
                    "end": e.span.end,
                    "replacement": e.replacement,
                })),
                "sources": p.sources.iter().map(|s| serde_json::json!({
                    "line": s.line,
                    "start": s.span.start,
                    "end": s.span.end,
                })).collect::<Vec<_>>(),
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
                    "fits": m.fits(pages_allowed(text, paper)),
                }),
            )
        })
        .collect();
    serde_json::json!({
        "file": file.display().to_string(),
        "fit": fit,
        "fits": fits,
        "measured": measured,
        "note": note,
        "proposals": proposals,
        "not_joined": not_joined,
    })
}

/// Typesets a song on the papers named and says what each made of it:
/// `tex::measure::measure` with the songbook and package bound, or, in a
/// test, a stand-in that counts lines.
type Measurer<'a> =
    dyn FnMut(&str, &[&str]) -> Result<BTreeMap<String, tex::measure::Measured>, String> + 'a;

/// Whether a change that lengthens lines makes the song worse on the
/// papers it lengthens them on (`checked_papers`, each already in
/// `measured`): a line it lengthens too wide already, or typeset with it,
/// more pages or more lines too wide. `None` when it does not, otherwise
/// why, in words.
fn too_wide(
    text: &str,
    measure: &mut Measurer,
    measured: &BTreeMap<String, tex::measure::Measured>,
    p: &tex::fit::Proposal,
) -> Result<Option<String>, String> {
    let papers = p.checked_papers();
    let mut wide: Vec<String> = papers
        .iter()
        .filter_map(|paper| measured.get(*paper))
        .flat_map(|m| &m.wraps)
        .filter(|w| p.lengthens(w.line, w.in_chorus))
        .map(|w| match w.line {
            Some(line) => format!("line {line}"),
            None => "a line".to_string(),
        })
        .collect();
    wide.dedup();
    if !wide.is_empty() {
        return Ok(Some(format!(
            "{} too wide already, and joined it would be longer",
            wide.join(", ")
        )));
    }
    let changed = tex::fit::apply(text, &[p])?;
    let after = measure(&changed, papers)?;
    // The papers a lengthened line is too wide for, and those it takes
    // more pages on.
    let (mut wide, mut longer) = (Vec::new(), Vec::new());
    for (paper, m) in &after {
        let before = &measured[paper];
        let count = |m: &tex::measure::Measured, chorus: bool| {
            m.wraps.iter().filter(|w| w.in_chorus == chorus).count()
        };
        let paper = paper.to_uppercase();
        if m.pages > before.pages {
            longer.push(paper.clone());
        }
        if count(m, false) > count(before, false) || count(m, true) > count(before, true) {
            wide.push(paper);
        }
    }
    let mut worse = Vec::new();
    if !wide.is_empty() {
        worse.push(format!(
            "a joined line is too wide for {}",
            wide.join(" and ")
        ));
    }
    if !longer.is_empty() {
        worse.push(format!(
            "the song takes more pages on {}",
            longer.join(" and ")
        ));
    }
    Ok((!worse.is_empty()).then(|| format!("joined, {}", worse.join(", and "))))
}

/// One change `shrink --apply` made: the proposal as it was listed when it
/// was taken.
#[derive(Debug, Clone, PartialEq)]
struct Step {
    id: String,
    kind: &'static str,
    /// The line it was on in the song as it stood when it was made.
    line: usize,
    description: String,
    score: u8,
    grade: &'static str,
    /// The part it acts on, by its kind and its number among the parts of
    /// that kind, which is how it is found again (`replay`).
    part: Option<(String, usize)>,
}

impl Step {
    fn of(p: &tex::fit::Proposal, text: &str) -> Step {
        Step {
            id: p.id.clone(),
            kind: p.kind,
            line: p.line,
            part: part_of(text, p.line),
            description: p.description.clone(),
            score: p.score,
            grade: p.grade(),
        }
    }
}

/// The part a line of the song is in: its kind and its number among the
/// parts of that kind, from 0. `None` outside every part (the prelude).
fn part_of(text: &str, line: usize) -> Option<(String, usize)> {
    let parts = tex::scan(text).parts;
    let last_line = |span: &std::ops::Range<usize>| part_line(text, span.end);
    let at = parts
        .iter()
        .position(|p| p.line <= line && line <= last_line(&p.span))?;
    let kind = parts[at].kind.clone();
    let number = parts[..at].iter().filter(|p| p.kind == kind).count();
    Some((kind, number))
}

/// The 1-based line a byte of the song is on.
fn part_line(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// Why `shrink --apply` stopped.
#[derive(Debug, Clone, PartialEq)]
enum Stopped {
    /// The song fits the paper.
    Fits,
    /// No change is left that helps.
    NothingLeft,
    /// Only a last resort is left (a smaller type, chords on the first part
    /// of a kind alone), and `--last-resort` did not allow it: this one.
    LastResort(Step),
}

impl Stopped {
    fn name(&self) -> &'static str {
        match self {
            Stopped::Fits => "fits",
            Stopped::NothingLeft => "nothing_left",
            Stopped::LastResort(_) => "last_resort",
        }
    }
}

/// What `shrink --apply` made of a song.
struct Fitted {
    text: String,
    steps: Vec<Step>,
    stopped: Stopped,
    /// The song as it ends, measured on the papers looked at.
    measured: BTreeMap<String, tex::measure::Measured>,
}

/// A song needs fewer changes than this, by far; the limit only keeps a
/// change that undoes itself from going round for ever.
const MAX_STEPS: usize = 40;

/// Where a song stands against the paper it is fitted to: whether it fits,
/// and if not, the stages whose changes help, as `shrink --fit` lists them.
/// A5 is helped by the changes for every paper first while A4 overflows (A5
/// overflows then too), then by those for small paper.
fn fit_state(
    text: &str,
    paper: &str,
    measure: &mut Measurer,
    measured: &mut BTreeMap<String, tex::measure::Measured>,
) -> Result<Option<Vec<tex::fit::Stage>>, String> {
    use tex::fit::Stage;
    let mut on = |paper: &str| -> Result<tex::measure::Measured, String> {
        if let Some(m) = measured.get(paper) {
            return Ok(m.clone());
        }
        let m = measure(text, &[paper])?
            .remove(paper)
            .ok_or_else(|| format!("No measurement for {}", paper))?;
        measured.insert(paper.to_string(), m.clone());
        Ok(m)
    };
    if !on("a4")?.fits(1) {
        return Ok(Some(if paper == "a4" {
            vec![Stage::AllPapers]
        } else {
            vec![Stage::AllPapers, Stage::SmallPaper]
        }));
    }
    if paper == "a4" || on("a5")?.fits(pages_allowed(text, "a5")) {
        return Ok(None);
    }
    Ok(Some(vec![Stage::SmallPaper]))
}

/// Fits a song to a paper the way a person following `shrink` would: the
/// first proposal taken, the song tidied and measured again, until it fits.
/// The proposals are ranked so that taking the first each time never leads
/// into a corner (`tex::fit`), and every step is measured, since the
/// measurement says how many pages, not by how much the song overflows.
///
/// A change that lengthens lines (the second voice inline, joined lines) is
/// taken only once typesetting it shows no line too wide. A last resort is
/// taken only when `last_resort` allows; otherwise the fitting stops before
/// it and says which, as the song spec leaves that to a person.
///
/// Once the song fits, each change but the last is tried left out, latest
/// first: the song made again from the start without it, by the same
/// changes found afresh. One a later change made unnecessary goes, so the
/// song keeps as much of itself as the paper allows, but only when the
/// changes after it read as well (score as high or higher): a chorus
/// reminder is not given back for a smaller type that would fit alone.
fn shrink_to_fit(
    original: &str,
    paper: &str,
    last_resort: bool,
    measure: &mut Measurer,
) -> Result<Fitted, String> {
    let mut text = original.to_string();
    let mut steps = Vec::new();
    let mut measured = BTreeMap::new();
    let stopped = loop {
        measured.clear();
        let Some(stages) = fit_state(&text, paper, measure, &mut measured)? else {
            break Stopped::Fits;
        };
        if steps.len() >= MAX_STEPS {
            break Stopped::NothingLeft;
        }
        let all = tex::fit::proposals(&text);
        let mut held_back = None;
        let mut chosen = None;
        for p in all
            .iter()
            .filter(|p| !p.is_correction() && stages.contains(&p.stage))
        {
            if p.grade() == "last resort" && !last_resort {
                held_back.get_or_insert_with(|| Step::of(p, &text));
                continue;
            }
            if p.needs_width_check() {
                for paper in p.checked_papers() {
                    if !measured.contains_key(*paper) {
                        let m = measure(&text, &[paper])?;
                        measured.extend(m);
                    }
                }
                if too_wide(&text, measure, &measured, p)?.is_some() {
                    continue;
                }
            }
            chosen = Some(p);
            break;
        }
        let Some(p) = chosen else {
            break held_back.map_or(Stopped::NothingLeft, Stopped::LastResort);
        };
        steps.push(Step::of(p, &text));
        text = tex::tidy::tidy(&tex::fit::apply(&text, &[p])?).0;
    };

    if stopped == Stopped::Fits && steps.len() > 1 {
        for left_out in (0..steps.len() - 1).rev() {
            // Never a change traded for a less readable one: the smaller
            // type alone may fit, but the chords hidden before it read
            // better.
            if steps[left_out + 1..]
                .iter()
                .any(|later| later.score < steps[left_out].score)
            {
                continue;
            }
            let mut fewer = steps.clone();
            fewer.remove(left_out);
            let Some((again, made)) = replay(original, &fewer)? else {
                continue;
            };
            let mut m = BTreeMap::new();
            if fit_state(&again, paper, measure, &mut m)?.is_none() {
                steps = made;
                text = again;
                measured = m;
            }
        }
    }
    Ok(Fitted {
        text,
        steps,
        stopped,
        measured,
    })
}

/// The song made again from `original` by these changes, each found afresh
/// by its kind and the part it acts on (its lines and its wording move when
/// a change before it is left out, the part does not), and the steps as
/// they were made. `None` when one is not offered
/// any more without the change left out.
fn replay(original: &str, steps: &[Step]) -> Result<Option<(String, Vec<Step>)>, String> {
    let mut text = original.to_string();
    let mut made = Vec::new();
    for step in steps {
        let all = tex::fit::proposals(&text);
        let Some(p) = all
            .iter()
            .find(|p| p.kind == step.kind && part_of(&text, p.line) == step.part)
        else {
            return Ok(None);
        };
        made.push(Step::of(p, &text));
        text = tex::tidy::tidy(&tex::fit::apply(&text, &[p])?).0;
    }
    Ok(Some((text, made)))
}

/// What `shrink --apply --json` prints. Its shape is part of the interface
/// like the listing's (`the_fit_json_keeps_its_shape` pins it): `fits`,
/// `stopped` (`fits`, `nothing_left`, `last_resort`), the last resort held
/// back, the changes made (or, when it does not fit, tried and not made)
/// and the pages measured last.
fn fit_json(name: &str, paper: &str, fitted: &Fitted) -> serde_json::Value {
    let step_json = |s: &Step| {
        serde_json::json!({
            "id": s.id,
            "kind": s.kind,
            "line": s.line,
            "description": s.description,
            "score": s.score,
            "grade": s.grade,
        })
    };
    let measured: serde_json::Map<String, serde_json::Value> = fitted
        .measured
        .iter()
        .map(|(p, m)| {
            (
                p.clone(),
                serde_json::json!({
                    "pages": m.pages,
                    "fits": m.fits(pages_allowed(&fitted.text, p)),
                }),
            )
        })
        .collect();
    serde_json::json!({
        "file": name,
        "fit": paper,
        "fits": fitted.stopped == Stopped::Fits,
        "stopped": fitted.stopped.name(),
        "last_resort": match &fitted.stopped {
            Stopped::LastResort(s) => step_json(s),
            _ => serde_json::Value::Null,
        },
        "steps": fitted.steps.iter().map(step_json).collect::<Vec<_>>(),
        "measured": measured,
    })
}

/// `shrink --apply`: fits the song, writes it where `output` says, lists
/// each change made on stderr as `<file>:<line>: <description>`, and says
/// how it ended (with `--json`, on stdout as JSON). A song it cannot fit is
/// written as it was: the changes tried are listed, none is made.
#[allow(clippy::too_many_arguments)]
fn shrink_file(
    file: &Path,
    paper: &str,
    last_resort: bool,
    songbook: Option<&Path>,
    package: Option<&Path>,
    json: bool,
    output: &Output,
) -> Result<(), String> {
    let text = fs::read_to_string(file)
        .map_err(|e| format!("Failed to read file '{}': {}", file.display(), e))?;
    let default_songbook = PathBuf::from("../ZubuvZpevnik");
    let songbook = songbook.unwrap_or(&default_songbook);
    let fitted = shrink_to_fit(
        &text,
        paper,
        last_resort,
        &mut |t: &str, papers: &[&str]| tex::measure::measure(songbook, package, t, papers),
    )?;
    let name = file.display().to_string();
    let fits = fitted.stopped == Stopped::Fits;
    // All or nothing: chords hidden without the song fitting cost the
    // reader and buy nothing, so a song that still does not fit is left as
    // it was, and what was tried is only listed.
    for step in &fitted.steps {
        let tried = if fits { "" } else { "tried, not made: " };
        eprintln!("{}:{}: {}{}", name, step.line, tried, step.description);
    }
    let song = if fits { &fitted.text } else { &text };
    write_back(file, song, fits && !fitted.steps.is_empty(), output)?;

    let paper_name = paper.to_uppercase();
    let changes = match fitted.steps.len() {
        1 => "1 change".to_string(),
        n => format!("{n} changes"),
    };
    if json {
        let out = fit_json(&name, paper, &fitted);
        return write_stdout(&format!("{out}\n"));
    }
    match &fitted.stopped {
        Stopped::Fits if fitted.steps.is_empty() => {
            eprintln!("It fits {paper_name} as it is.")
        }
        Stopped::Fits => eprintln!(
            "It fits {paper_name} after {changes}. Record it in the songbook with:\n    zubuvzpevnik/scripts/checkpaper.py --measure {paper} --write <song>"
        ),
        Stopped::NothingLeft => eprintln!(
            "Warning: it does not fit {paper_name} even after {changes}, and nothing is left to propose; the song is left as it was."
        ),
        Stopped::LastResort(s) => eprintln!(
            "Warning: it does not fit {paper_name} without a last resort, even after {changes}; the song is left as it was. Next would be {}: {}. Allow it with --last-resort.",
            s.id, s.description
        ),
    }
    Ok(())
}

/// `join`'s kinds of part left as they are, with the reason.
fn push_not_joined(out: &mut String, not_joined: &[String]) {
    if !not_joined.is_empty() {
        out.push_str("  Not joined:\n");
        for why in not_joined {
            out.push_str(&format!("    {why}\n"));
        }
    }
}

/// Applies the chosen proposals to a song. The proposals are found afresh
/// in the file as it is now, so an id from an older listing that no longer
/// applies is refused rather than applied to the wrong bytes.
fn apply_file(
    file: &Path,
    ids: &[String],
    dismiss: &[String],
    output: &Output,
) -> Result<(), String> {
    let text = fs::read_to_string(file)
        .map_err(|e| format!("Failed to read file '{}': {}", file.display(), e))?;
    let all = tex::fit::proposals(&text);
    let find = |id: &String| {
        all.iter().find(|p| &p.id == id).ok_or_else(|| {
            format!(
                "No proposal '{}' for '{}' (ask again with shrink, join or check).",
                id,
                file.display()
            )
        })
    };
    if !dismiss.is_empty() {
        let mut edits = Vec::new();
        for id in dismiss {
            let p = find(id)?;
            let edit = tex::fit::dismissal(&text, p)
                .ok_or_else(|| format!("'{}' is no finding of check to mark right.", id))?;
            eprintln!(
                "{}:{}: marked right: {}",
                file.display(),
                p.line,
                p.description
            );
            edits.push(edit);
        }
        edits.sort_by_key(|e| e.span.start);
        edits.dedup_by_key(|e| (e.span.start, e.replacement.clone()));
        let mut changed = text.clone();
        for edit in edits.iter().rev() {
            changed.replace_range(edit.span.clone(), &edit.replacement);
        }
        return write_back(file, &changed, true, output);
    }
    let mut chosen = Vec::new();
    for id in ids {
        chosen.push(find(id)?);
    }
    let changed = tex::fit::apply(&text, &chosen)?;
    for p in &chosen {
        eprintln!("{}:{}: {}", file.display(), p.line, p.description);
    }
    write_back(file, &changed, true, output)
}

const USAGE: &str = "\
Usage:
  chords_converter --version | --help
  chords_converter <SOURCE> [FORMAT]
  chords_converter convert <SOURCE> [--format FORMAT] [--tidy] [-o FILE | -o -]
  chords_converter tidy <SONG.tex> [-o FILE | -o -]
  chords_converter shrink <SONG.tex> [--fit a4|a5] [--measure] [--songbook DIR] [--package DIR] [--json]
  chords_converter shrink <SONG.tex> --apply [--fit a4|a5] [--last-resort] [--songbook DIR] [--package DIR] [--json] [-o FILE | -o -]
  chords_converter join <SONG.tex> [--songbook DIR] [--package DIR] [--json]
  chords_converter check <SONG.tex> [--json]
  chords_converter apply <SONG.tex> --id ID [--id ID ...] [-o FILE | -o -]
  chords_converter apply <SONG.tex> --dismiss ID [--dismiss ID ...] [-o FILE | -o -]

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

shrink (until 0.6.0 suggest, which still works) lists changes that make
a song shorter, for a person to accept or reject, rated by how readable they leave the song (0-100: good, fair,
poor, last resort) and most readable first: a chorus shortened to its
reminder line, chords hidden inside a part whose lines repeat its first
lines' chords, chords hidden where they repeat ones shown just before
(the nearer, the better, and late in the song rather than early), a
smaller type on small paper, and with --measure the second voice inline on
A4 where no line gets too wide.
--fit a4|a5 keeps to the ones that help fit that paper; --measure typesets
the song with the songbook's own checkpaper.py (the songbook is
../ZubuvZpevnik unless --songbook says otherwise; its package and scripts are
its submodule zubuvzpevnik/, or the package --package names) and says
whether it fits.
shrink --apply fits the song itself: it takes the first proposal, tidies
and measures the song again, until it fits the --fit paper (a5 unless
given; A5 means A4 as well). Line-lengthening changes and joined lines are
taken once typesetting shows no line too wide. It stops before a last
resort (a smaller type, chords on the first part of a kind alone) unless
--last-resort allows it. Once it fits, a change made unnecessary by a later
one as readable is left out. A song it cannot fit is left as it was, and
the changes tried are listed. Each change is listed on stderr as
<file>:<line>: <description>; --json prints the outcome on stdout.
join offers every two lines of each part of a kind joined into one, where
every part of the kind can take it (an odd last line stays alone when every
part has as many lines), after typesetting the joined song to check no line
comes out too wide on A4 or A5 (slow: a run per kind); it says why a kind
is not joined.
check lists corrections, whether the song fits or not: a part with the
chorus's words typed as something else may be made a chorus (then tidy
reprints its copies); a line's copies differing in one word by a slip;
apostrophes typed both ways; ~ in an English song; a chord at a line's
end in one part and at the next line's start in the others; a comma missing
before a Czech or Slovak clause. A line '% nocheck KIND ...' above a line
marks its findings of those kinds right (apply --dismiss ID adds it; a bare
'% nocheck' marks all of them); for apostrophes and ties, which are about the
whole song, the mark may stand anywhere.
apply makes the chosen ones, from any of the three; apply one, then ask
again.";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    Help,
    /// The tool's version, for a caller checking it is new enough: it goes up
    /// whenever the command line or the `shrink --json` format changes.
    Version,
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
        package: Option<PathBuf>,
        json: bool,
    },
    /// `shrink --apply`: the first proposal taken, again and again, until
    /// the song fits the paper.
    Fit {
        file: PathBuf,
        paper: String,
        last_resort: bool,
        songbook: Option<PathBuf>,
        package: Option<PathBuf>,
        json: bool,
        output: Output,
    },
    /// Corrections a person judges: a part that may be the chorus.
    Check {
        file: PathBuf,
        json: bool,
    },
    /// Lines joined in pairs, each checked by typesetting the joined song.
    Join {
        file: PathBuf,
        songbook: Option<PathBuf>,
        package: Option<PathBuf>,
        json: bool,
    },
    Apply {
        file: PathBuf,
        ids: Vec<String>,
        /// Findings of `check` marked right instead (`% nocheck`).
        dismiss: Vec<String>,
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
        "-V" | "--version" | "version" => Ok(Command::Version),
        "convert" => parse_convert(&args[1..]),
        "tidy" => parse_tidy(&args[1..]),
        "join" => match parse_suggest(&args[1..])? {
            Command::Suggest {
                file,
                fit: None,
                songbook,
                package,
                json,
                ..
            } => Ok(Command::Join {
                file,
                songbook,
                package,
                json,
            }),
            _ => Err("join takes no --fit: it checks A4 and A5 both.".to_string()),
        },
        // `suggest` was its name until 0.6.0; callers may still use it.
        "shrink" | "suggest" => parse_suggest(&args[1..]),
        "check" => match parse_suggest(&args[1..])? {
            Command::Suggest {
                file,
                fit: None,
                measure: false,
                songbook: None,
                package: None,
                json,
            } => Ok(Command::Check { file, json }),
            _ => Err("check takes only a song file and --json.".to_string()),
        },
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
    let mut package = None;
    let mut json = false;
    let mut apply = false;
    let mut last_resort = false;
    let mut output = None;
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
            "--package" => package = Some(PathBuf::from(value()?)),
            "--json" => json = true,
            "--apply" => apply = true,
            "--last-resort" => last_resort = true,
            "--output" | "-o" => {
                output = Some(match value()?.as_str() {
                    "-" => Output::Stdout,
                    path => Output::File(PathBuf::from(path)),
                })
            }
            option if option.starts_with('-') => {
                return Err(format!("Unknown option '{}'.", option));
            }
            _ if file.is_some() => return Err(format!("Unexpected argument '{}'.", arg)),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    let file = file.ok_or_else(|| "A song file is needed.".to_string())?;
    if apply {
        let output = output.unwrap_or(Output::Default);
        if json && output == Output::Stdout {
            return Err("--json and -o - would both go to standard output.".to_string());
        }
        // Fitting needs to know when the song fits: it always measures.
        return Ok(Command::Fit {
            file,
            paper: fit.unwrap_or_else(|| "a5".to_string()),
            last_resort,
            songbook,
            package,
            json,
            output,
        });
    }
    if last_resort || output.is_some() {
        return Err("--last-resort and -o go with --apply.".to_string());
    }
    Ok(Command::Suggest {
        file,
        fit,
        measure,
        songbook,
        package,
        json,
    })
}

fn parse_apply(args: &[String]) -> Result<Command, String> {
    let mut file = None;
    let mut ids = Vec::new();
    let mut dismiss = Vec::new();
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
            "--dismiss" => dismiss.push(value()?),
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
    if ids.is_empty() == dismiss.is_empty() {
        return Err("apply needs --id or --dismiss (not both).".to_string());
    }
    Ok(Command::Apply {
        file,
        ids,
        dismiss,
        output,
    })
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
        Command::Version => {
            println!("chords_converter {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Command::Tidy { file, output } => tidy_file(&file, &output),
        Command::Suggest {
            file,
            fit,
            measure,
            songbook,
            package,
            json,
        } => list_proposals(
            &file,
            fit.as_deref(),
            measure,
            songbook.as_deref(),
            package.as_deref(),
            json,
            Listing::Shrink,
        ),
        Command::Join {
            file,
            songbook,
            package,
            json,
        } => list_proposals(
            &file,
            None,
            true,
            songbook.as_deref(),
            package.as_deref(),
            json,
            Listing::Join,
        ),
        Command::Fit {
            file,
            paper,
            last_resort,
            songbook,
            package,
            json,
            output,
        } => shrink_file(
            &file,
            &paper,
            last_resort,
            songbook.as_deref(),
            package.as_deref(),
            json,
            &output,
        ),
        Command::Check { file, json } => {
            list_proposals(&file, None, false, None, None, json, Listing::Check)
        }
        Command::Apply {
            file,
            ids,
            dismiss,
            output,
        } => apply_file(&file, &ids, &dismiss, &output),
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

    /// ZubuvZpevnikar runs `chords_converter <source> latex`; that form has to
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
                package: None,
                json: true,
            })
        );
        assert!(parse_args(&args(&["suggest", "S.tex", "--fit", "letter"])).is_err());
        assert_eq!(
            parse_args(&args(&[
                "shrink",
                "S.tex",
                "--apply",
                "--last-resort",
                "-o",
                "-"
            ])),
            Ok(Command::Fit {
                file: "S.tex".into(),
                paper: "a5".into(),
                last_resort: true,
                songbook: None,
                package: None,
                json: false,
                output: Output::Stdout,
            })
        );
        assert!(matches!(
            parse_args(&args(&["shrink", "S.tex", "--apply", "--fit", "a4"])),
            Ok(Command::Fit { paper, .. }) if paper == "a4"
        ));
        assert!(parse_args(&args(&["shrink", "S.tex", "--apply", "--json", "-o", "-"])).is_err());
        assert!(parse_args(&args(&["shrink", "S.tex", "--last-resort"])).is_err());
        assert!(parse_args(&args(&["shrink", "S.tex", "-o", "x.tex"])).is_err());
        assert!(parse_args(&args(&["suggest"])).is_err());
        // shrink is the name now; suggest stays for older callers.
        assert_eq!(
            parse_args(&args(&["shrink", "S.tex"])),
            parse_args(&args(&["suggest", "S.tex"]))
        );
        assert_eq!(
            parse_args(&args(&["check", "S.tex", "--json"])),
            Ok(Command::Check {
                file: "S.tex".into(),
                json: true,
            })
        );
        assert!(parse_args(&args(&["check", "S.tex", "--measure"])).is_err());
        assert_eq!(
            parse_args(&args(&["join", "S.tex", "--json"])),
            Ok(Command::Join {
                file: "S.tex".into(),
                songbook: None,
                package: None,
                json: true,
            })
        );
        assert!(parse_args(&args(&["join", "S.tex", "--fit", "a5"])).is_err());
        assert_eq!(
            parse_args(&args(&["join", "S.tex", "--package", "P", "--json"])),
            Ok(Command::Join {
                file: "S.tex".into(),
                songbook: None,
                package: Some("P".into()),
                json: true,
            })
        );
        assert!(parse_args(&args(&["check", "S.tex", "--package", "P"])).is_err());

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
                dismiss: Vec::new(),
                output: Output::Stdout,
            })
        );
        assert!(parse_args(&args(&["apply", "S.tex"])).is_err());
        assert_eq!(
            parse_args(&args(&["apply", "S.tex", "--dismiss", "typo@20"])),
            Ok(Command::Apply {
                file: "S.tex".into(),
                ids: Vec::new(),
                dismiss: vec!["typo@20".to_string()],
                output: Output::Default,
            })
        );
        assert!(
            parse_args(&args(&[
                "apply",
                "S.tex",
                "--id",
                "a@1",
                "--dismiss",
                "b@2"
            ]))
            .is_err()
        );
    }

    /// The whole of fetching stays under ZubuvZpevnikar's 60 s: a site
    /// that lets every request run into its timeout gets a second try cut
    /// to what is left of the budget, and no third.
    #[test]
    fn fetching_keeps_within_its_budget() {
        let s = Duration::from_secs;
        assert_eq!(next_try(1, s(0)), Some((s(0), s(30))));
        // Turned away at once: the pauses grow, the timeout stays.
        assert_eq!(next_try(2, s(1)), Some((s(2), s(30))));
        assert_eq!(next_try(3, s(4)), Some((s(4), s(30))));
        assert_eq!(next_try(4, s(10)), None);
        // Every request runs into its timeout: 30 s, then 2 s and 18 s.
        assert_eq!(next_try(2, s(30)), Some((s(2), s(18))));
        assert_eq!(next_try(3, s(50)), None);
        // Too little left for a try worth making.
        assert_eq!(next_try(2, s(44)), None);
        assert_eq!(next_try(2, s(43)), Some((s(2), s(5))));
    }

    #[test]
    fn a_song_marked_for_a_spread_may_take_two_a5_pages() {
        assert_eq!(pages_allowed("    pap={a4,a5x2},\n", "a5"), 2);
        assert_eq!(pages_allowed("    pap={a4,a5},\n", "a5"), 1);
        assert_eq!(pages_allowed("    % pap={a5x2},\n", "a5"), 1);
        assert_eq!(pages_allowed("    pap={a4,a5x2},\n", "a4"), 1);
    }

    #[test]
    fn version_and_help_are_not_sources() {
        for arg in ["--version", "-V", "version"] {
            assert_eq!(parse_args(&args(&[arg])), Ok(Command::Version), "{arg}");
        }
        for arg in ["--help", "-h", "help"] {
            assert_eq!(parse_args(&args(&[arg])), Ok(Command::Help), "{arg}");
        }
    }

    /// What `--json` prints, field by field: ZubuvZpevnikar reads this
    /// shape, so a change to it is a change of the interface (CLAUDE.md),
    /// with a version bump and a note to the UI. The offsets are bytes
    /// into the song; `measured` is as the typesetter would report it.
    #[test]
    fn the_json_listing_keeps_its_shape() {
        use serde_json::json;
        use tex::measure::{Measured, Wrap};
        let text = "\\beginsong{T}[\n    lang={czech},\n]\n\n\
            \\beginverse\n    \\[C]vím že \\[G]přijdeš\n\\endverse\n\n\
            \\beginchorus\n    \\[F]Refrén \\[G]zní\n\\endchorus\n\n\
            \\beginverse\n    \\[Am]a vím \\[E]zase\n\\endverse\n\n\
            \\beginchorus\n    \\[F]Refrén \\[G]zní\n\\endchorus\n\n\\endsong\n";
        let all = tex::fit::proposals(text);

        // `shrink --fit a5 --measure --json`, on a song that fits A4.
        let shrink: Vec<&tex::fit::Proposal> = all
            .iter()
            .filter(|p| !p.needs_width_check() && !p.is_correction())
            .collect();
        let measured = BTreeMap::from([(
            "a5".to_string(),
            Measured {
                pages: 2,
                wraps: vec![
                    Wrap {
                        line: Some(6),
                        in_chorus: false,
                    },
                    Wrap {
                        line: None,
                        in_chorus: true,
                    },
                ],
            },
        )]);
        let listing = json_listing(
            Path::new("Song.tex"),
            text,
            Some("a5"),
            Some(false),
            &measured,
            "It fits A4: the changes for small paper only.",
            &shrink,
            &["verse: one line each, nothing to pair".to_string()],
        );
        let first_chorus = json!({"line": 9, "start": 90, "end": 138});
        assert_eq!(
            listing,
            json!({
                "file": "Song.tex",
                "fit": "a5",
                "fits": false,
                "measured": {
                    "a5": {
                        "pages": 2,
                        "fits": false,
                        "wraps": [
                            {"line": 6, "in_chorus": false},
                            {"line": null, "in_chorus": true},
                        ],
                    },
                },
                "note": "It fits A4: the changes for small paper only.",
                "not_joined": ["verse: one line each, nothing to pair"],
                "proposals": [
                    {
                        "id": "insertchorus*@17",
                        "kind": "insertchorus*",
                        "stage": 1,
                        "line": 17,
                        "description": "the 2nd chorus has the words of the 1st chorus: shorten it to \\insertchorus*",
                        "gain_lines": 1,
                        "score": 80,
                        "grade": "good",
                        "category": "fit",
                        "dismiss": null,
                        "sources": [first_chorus],
                        "edits": [{"start": 188, "end": 236, "replacement": "\\insertchorus*"}],
                    },
                    {
                        "id": "chordsoff@17",
                        "kind": "chordsoff",
                        "stage": 1,
                        "line": 17,
                        "description": "the 2nd chorus repeats the chords of the 1st chorus (1 line away): \\chordsoff",
                        "gain_lines": 1,
                        "score": 68,
                        "grade": "fair",
                        "category": "fit",
                        "dismiss": null,
                        "sources": [first_chorus],
                        "edits": [{"start": 200, "end": 200, "replacement": "\\chordsoff"}],
                    },
                    {
                        "id": "chordsoffonsmallpage@17",
                        "kind": "chordsoffonsmallpage",
                        "stage": 2,
                        "line": 17,
                        "description": "the 2nd chorus repeats the chords of the 1st chorus (1 line away): \\chordsoffonsmallpage",
                        "gain_lines": 1,
                        "score": 68,
                        "grade": "fair",
                        "category": "fit",
                        "dismiss": null,
                        "sources": [first_chorus],
                        "edits": [{"start": 200, "end": 200, "replacement": "\\chordsoffonsmallpage"}],
                    },
                    {
                        "id": "smalleronsmallpage@5",
                        "kind": "smalleronsmallpage",
                        "stage": 2,
                        "line": 5,
                        "description": "set the song a size smaller on small paper (8 pt): \\smalleronsmallpage",
                        "gain_lines": 1,
                        "score": 20,
                        "grade": "last resort",
                        "category": "fit",
                        "dismiss": null,
                        "sources": [],
                        "edits": [{"start": 36, "end": 36, "replacement": "\\smalleronsmallpage\n\n"}],
                    },
                ],
            })
        );

        // `check --json`: a correction, with the edit that marks it right.
        let corrections: Vec<&tex::fit::Proposal> =
            all.iter().filter(|p| p.is_correction()).collect();
        let listing = json_listing(
            Path::new("Song.tex"),
            text,
            None,
            None,
            &BTreeMap::new(),
            "",
            &corrections,
            &[],
        );
        assert_eq!(listing["fit"], json!(null));
        assert_eq!(listing["fits"], json!(null));
        assert_eq!(listing["measured"], json!({}));
        assert_eq!(listing["not_joined"], json!([]));
        assert_eq!(
            listing["proposals"],
            json!([{
                "id": "comma@6",
                "kind": "comma",
                "stage": 1,
                "line": 6,
                "description": "verse 1: \"vím že\" may want a comma before \"že\"",
                "gain_lines": 0,
                "score": 0,
                "grade": "correction",
                "category": "typo",
                "dismiss": {"start": 48, "end": 48, "replacement": "    % nocheck comma\n"},
                "sources": [],
                "edits": [{"start": 60, "end": 60, "replacement": ","}],
            }])
        );
    }

    /// A song of three verses and three choruses, the last one longer,
    /// made up for the tests: 38 printed lines, chord rows counted.
    const PAPER_BOATS: &str = "\\beginsong{Paper Boats}[\n    by={Nobody},\n    lang={english},\n    pap={},\n]\n\n\\beginverse\n    \\[G]Paper boats are \\[C]sailing\n    \\[D]down the river \\[G]slow\n    \\[G]Paper boats are \\[C]failing\n    \\[D]where the waters \\[G]go\n\\endverse\n\n\\beginchorus\n    \\[C]Fold another \\[G]one\n    \\[D]fold it in the \\[G]sun\n\\endchorus\n\n\\beginverse\n    \\[G]Morning on the \\[C]water\n    \\[D]evening on the \\[G]shore\n    \\[G]Every little \\[C]daughter\n    \\[D]wants a boat once \\[G]more\n\\endverse\n\n\\beginchorus\n    \\[C]Fold another \\[G]one\n    \\[D]fold it in the \\[G]sun\n\\endchorus\n\n\\beginverse\n    \\[G]Lanterns on the \\[C]harbour\n    \\[D]lighting up the \\[G]bay\n    \\[G]Sailing ever \\[C]farther\n    \\[D]paper boats a\\[G]way\n\\endverse\n\n\\beginchorus\n    \\[C]Fold another \\[G]one\n    \\[D]fold it in the \\[G]sun\n    \\[C]fold another \\[G]one\n\\endchorus\n\n\\endsong\n";

    /// Typesets nothing: a page holds `a4` or `a5` printed lines, a line of
    /// words is one and its chord row another, unless the chords are off
    /// there; a reminder is one line, a full reprint four, and the smaller
    /// type on small paper takes a quarter off.
    fn lines_per_page(
        a4: usize,
        a5: usize,
    ) -> impl FnMut(&str, &[&str]) -> Result<BTreeMap<String, tex::measure::Measured>, String> {
        move |text: &str, papers: &[&str]| {
            let mut out = BTreeMap::new();
            for paper in papers {
                let small = *paper == "a5";
                let switch_off = |code: &str| {
                    code.contains("\\chordsoffonsmallpage") && small
                        || code
                            .split("\\chordsoff")
                            .skip(1)
                            .any(|rest| !rest.starts_with(|c: char| c.is_ascii_alphabetic()))
                };
                let mut weight = 0;
                for part in tex::scan(text).parts {
                    let mut off = switch_off(&text[part.opening_rest.clone()]);
                    for line in text[part.body.clone()].lines().map(str::trim) {
                        if line.is_empty() {
                        } else if line.starts_with("\\chordson") && !line.contains("\\[") {
                            off = false;
                        } else if line.starts_with("\\chordsoff") {
                            off = off || switch_off(line);
                        } else {
                            weight += 1 + usize::from(line.contains("\\[") && !off);
                        }
                    }
                }
                for line in text.lines().map(str::trim) {
                    if line.starts_with("\\insertchorus*") {
                        weight += 1;
                    } else if line.starts_with("\\insertchorus") {
                        weight += 4;
                    }
                }
                if small && text.contains("\\smalleronsmallpage") {
                    weight = weight * 3 / 4;
                }
                let lines = if small { a5 } else { a4 };
                out.insert(
                    paper.to_string(),
                    tex::measure::Measured {
                        pages: weight.div_ceil(lines).max(1) as u32,
                        wraps: Vec::new(),
                    },
                );
            }
            Ok(out)
        }
    }

    fn fit_paper_boats(paper: &str, last_resort: bool, a4: usize, a5: usize) -> Fitted {
        shrink_to_fit(PAPER_BOATS, paper, last_resort, &mut lines_per_page(a4, a5)).unwrap()
    }

    fn kinds(fitted: &Fitted) -> Vec<&str> {
        fitted.steps.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn a_song_that_fits_is_left_as_it_is() {
        let fitted = fit_paper_boats("a5", false, 38, 38);
        assert_eq!(fitted.stopped, Stopped::Fits);
        assert!(fitted.steps.is_empty());
        assert_eq!(fitted.text, PAPER_BOATS);
    }

    #[test]
    fn a_song_fitting_a4_is_made_to_fit_a5_on_small_paper_alone() {
        let fitted = fit_paper_boats("a5", false, 38, 36);
        assert_eq!(fitted.stopped, Stopped::Fits);
        assert_eq!(kinds(&fitted), ["chordsoffonsmallpageinside"]);
        assert!(fitted.text.contains("    \\chordsoffonsmallpage\n"));
        assert!(!fitted.text.contains("\\chordsoff\n"));
    }

    #[test]
    fn a_song_overflowing_a4_takes_a_change_for_every_paper_first() {
        let fitted = fit_paper_boats("a5", false, 35, 35);
        assert_eq!(fitted.stopped, Stopped::Fits);
        // Lines joined in pairs are the most readable change of all.
        assert_eq!(kinds(&fitted), ["joinlines"]);
        assert!(fitted.steps.iter().all(|s| !s.kind.contains("onsmallpage")));
        let a4 = fit_paper_boats("a4", false, 35, 10);
        assert_eq!(a4.stopped, Stopped::Fits);
        assert_eq!(kinds(&a4), ["joinlines"]);
    }

    #[test]
    fn it_stops_before_a_last_resort_unless_allowed() {
        let fitted = fit_paper_boats("a5", false, 38, 10);
        let Stopped::LastResort(next) = &fitted.stopped else {
            panic!("{:?}", fitted.stopped);
        };
        assert_eq!(next.grade, "last resort");
        assert!(!fitted.steps.is_empty());
        assert!(!fitted.text.contains("\\smalleronsmallpage"));
        assert!(fitted.steps.iter().all(|s| s.grade != "last resort"));

        let allowed = shrink_to_fit(PAPER_BOATS, "a5", true, &mut lines_per_page(38, 10)).unwrap();
        assert!(allowed.steps.len() > fitted.steps.len());
        assert!(allowed.steps.iter().any(|s| s.grade == "last resort"));
    }

    #[test]
    fn a_readable_change_is_not_given_back_for_a_smaller_type() {
        // The smaller type alone would fit 20 lines of small paper here,
        // but the chords hidden before it read better: they stay.
        let fitted = shrink_to_fit(PAPER_BOATS, "a5", true, &mut lines_per_page(26, 20)).unwrap();
        assert_eq!(fitted.stopped, Stopped::Fits);
        let kinds = kinds(&fitted);
        assert_eq!(kinds.last(), Some(&"smalleronsmallpage"));
        assert!(kinds.len() > 2, "{kinds:?}");
    }

    #[test]
    fn the_changes_made_again_make_the_same_song() {
        let fitted = fit_paper_boats("a5", false, 26, 22);
        assert!(fitted.steps.len() > 1);
        let (again, made) = replay(PAPER_BOATS, &fitted.steps).unwrap().unwrap();
        assert_eq!(again, fitted.text);
        assert_eq!(made, fitted.steps);
        // Without the first, the others are still found, on their lines.
        assert!(replay(PAPER_BOATS, &fitted.steps[1..]).unwrap().is_some());
    }

    #[test]
    fn the_fit_json_keeps_its_shape() {
        // ZubuvZpevnikar reads it: a change here is a version bump.
        let fitted = fit_paper_boats("a5", false, 38, 10);
        let out = fit_json("S.tex", "a5", &fitted);
        let keys = |v: &serde_json::Value| -> Vec<String> {
            v.as_object().unwrap().keys().cloned().collect()
        };
        assert_eq!(
            keys(&out),
            [
                "file",
                "fit",
                "fits",
                "last_resort",
                "measured",
                "steps",
                "stopped"
            ]
        );
        assert_eq!(out["stopped"], "last_resort");
        assert_eq!(out["fits"], false);
        assert_eq!(
            keys(&out["steps"][0]),
            ["description", "grade", "id", "kind", "line", "score"]
        );
        assert_eq!(keys(&out["last_resort"]), keys(&out["steps"][0]));
        assert_eq!(keys(&out["measured"]["a5"]), ["fits", "pages"]);
    }

    #[test]
    fn fitting_twice_changes_nothing_more() {
        let once = fit_paper_boats("a5", false, 38, 32);
        assert_eq!(once.stopped, Stopped::Fits);
        let twice = shrink_to_fit(&once.text, "a5", false, &mut lines_per_page(38, 32)).unwrap();
        assert!(twice.steps.is_empty());
        assert_eq!(twice.text, once.text);
    }
}
