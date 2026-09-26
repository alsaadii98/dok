//! `dok logs` — interleaved, level-coloured, JSON-aware log tailing.

use anyhow::{Result, anyhow};
use bollard::container::LogOutput;
use bollard::query_parameters::LogsOptionsBuilder;
use futures_util::stream::{StreamExt, select_all};
use std::collections::HashMap;

use crate::dk;
use crate::fmt;
use crate::json;
use crate::theme::{self, *};

pub async fn run(
    wanted: Vec<String>,
    follow: bool,
    tail: String,
    timestamps: bool,
    grep: Option<String>,
) -> Result<()> {
    let docker = dk::connect()?;

    if crate::demo::enabled() {
        return demo_logs(timestamps, grep);
    }

    let running = dk::containers(&docker, false).await?;

    // Resolve the requested names/ids against running containers.
    let targets: Vec<String> = if wanted.is_empty() {
        running.iter().map(dk::name_of).collect()
    } else {
        let mut out = Vec::new();
        for w in &wanted {
            let hit = running.iter().find(|c| {
                let n = dk::name_of(c);
                n == *w
                    || n.contains(w.as_str())
                    || c.id.as_deref().is_some_and(|id| id.starts_with(w.as_str()))
            });
            match hit {
                Some(c) => out.push(dk::name_of(c)),
                None => return Err(anyhow!("no running container matches `{w}`")),
            }
        }
        out
    };

    if targets.is_empty() {
        if !json::enabled() {
            println!("{}", dim("no running containers to tail"));
        }
        return Ok(());
    }

    let width = targets.iter().map(|t| t.chars().count()).max().unwrap_or(0).min(24);
    let opts = LogsOptionsBuilder::default()
        .follow(follow)
        .stdout(true)
        .stderr(true)
        .tail(&tail)
        .timestamps(true) // always fetched; only shown when asked
        .build();

    let streams = targets.iter().map(|name| {
        let name = name.clone();
        docker.logs(&name, Some(opts.clone())).map(move |item| (name.clone(), item)).boxed()
    });

    let mut merged = select_all(streams);
    // Partial chunks are common on busy streams; buffer per container.
    let mut buffers: HashMap<String, String> = HashMap::new();

    while let Some((name, item)) = merged.next().await {
        let (text, is_err) = match item {
            Ok(LogOutput::StdErr { message }) => {
                (String::from_utf8_lossy(&message).into_owned(), true)
            }
            Ok(out) => (out.to_string(), false),
            Err(e) => {
                eprintln!("{} {}", c("!", p().red), dim(&format!("{name}: {e}")));
                continue;
            }
        };

        let buf = buffers.entry(name.clone()).or_default();
        buf.push_str(&text);
        while let Some(idx) = buf.find('\n') {
            let line: String = buf.drain(..=idx).collect();
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                continue;
            }
            if let Some(g) = &grep
                && !line.to_lowercase().contains(&g.to_lowercase())
            {
                continue;
            }
            if json::enabled() {
                json::emit_line(&line_json(&name, line, is_err));
            } else {
                println!("{}", render_line(&name, width, line, timestamps, is_err));
            }
        }
    }

    // Flush whatever never got its newline.
    for (name, buf) in buffers {
        if !buf.trim().is_empty() {
            if json::enabled() {
                json::emit_line(&line_json(&name, buf.trim_end(), false));
            } else {
                println!("{}", render_line(&name, width, buf.trim_end(), timestamps, false));
            }
        }
    }
    Ok(())
}

/// Demo mode replays a canned interleaved stream through the real renderer.
fn demo_logs(timestamps: bool, grep: Option<String>) -> Result<()> {
    let lines = crate::demo::logs();
    let width = lines.iter().map(|(n, _, _)| n.chars().count()).max().unwrap_or(0).min(24);
    let base = chrono::Utc::now() - chrono::Duration::seconds(lines.len() as i64 * 7);
    for (i, (name, body, is_err)) in lines.iter().enumerate() {
        if let Some(g) = &grep
            && !body.to_lowercase().contains(&g.to_lowercase())
        {
            continue;
        }
        let stamp = (base + chrono::Duration::seconds(i as i64 * 7))
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let line = format!("{stamp} {body}");
        if json::enabled() {
            json::emit_line(&line_json(name, &line, *is_err));
        } else {
            println!("{}", render_line(name, width, &line, timestamps, *is_err));
        }
    }
    Ok(())
}

/// The full RFC 3339 stamp docker prefixed, if it did. `split_timestamp`
/// shortens to a clock time for the table; a consumer needs the instant.
fn full_timestamp(line: &str) -> (Option<&str>, &str) {
    let Some((head, rest)) = line.split_once(' ') else { return (None, line) };
    if head.len() >= 20 && head.as_bytes().get(4) == Some(&b'-') && head.contains('T') {
        return (Some(head), rest);
    }
    (None, line)
}

/// One log line as an object. The timestamp is split off when docker sent
/// one, so a consumer does not have to re-parse what dok already parsed.
fn line_json(name: &str, line: &str, is_err: bool) -> serde_json::Value {
    let (ts, msg) = full_timestamp(line);
    serde_json::json!({
        "container": name,
        "stream": if is_err { "stderr" } else { "stdout" },
        "timestamp": match ts { Some(t) => t.into(), None => serde_json::Value::Null },
        "message": msg,
    })
}

fn render_line(name: &str, width: usize, line: &str, show_ts: bool, is_err: bool) -> String {
    // Docker's --timestamps puts an RFC3339 stamp first; peel it off either way.
    let (stamp, body) = split_timestamp(line);

    // The separator carries the stream: dim for stdout, red for stderr. Many
    // images (postgres, nginx) log everything to stderr, so a louder marker
    // would paint the whole screen red.
    let prefix = format!(
        "{} {}",
        c(&fmt::pad(&fmt::truncate(name, width), width), theme::hash_color(name)),
        c(g().sep, if is_err { p().red } else { p().gray })
    );

    let mut out = prefix;
    if show_ts && let Some(ts) = stamp {
        out.push(' ');
        out.push_str(&dim(&ts));
    }
    out.push(' ');
    out.push_str(&highlight(body));
    out
}

/// Split a leading RFC3339 timestamp into a compact `HH:MM:SS.mmm`.
fn split_timestamp(line: &str) -> (Option<String>, &str) {
    let Some((head, rest)) = line.split_once(' ') else { return (None, line) };
    // 2024-05-01T12:34:56.789012345Z
    if head.len() >= 20 && head.as_bytes()[4] == b'-' && head.contains('T') {
        let time = head.split('T').nth(1).unwrap_or("");
        let short: String = time.chars().take(12).collect();
        return (Some(short.trim_end_matches('.').to_string()), rest);
    }
    (None, line)
}

/// Colourise a log body: JSON objects get pretty key=value, plain text gets
/// its level token and any embedded HTTP status highlighted.
fn highlight(body: &str) -> String {
    let trimmed = body.trim_start();
    if trimmed.starts_with('{')
        && let Ok(serde_json::Value::Object(map)) = serde_json::from_str(trimmed)
    {
        return highlight_json(&map);
    }
    highlight_text(body)
}

fn highlight_json(map: &serde_json::Map<String, serde_json::Value>) -> String {
    // Lead with the fields humans scan for, then everything else.
    const LEAD: [&str; 6] = ["level", "severity", "lvl", "time", "ts", "timestamp"];
    const MSG: [&str; 4] = ["msg", "message", "event", "error"];

    let mut out = String::new();
    if let Some(level) = LEAD.iter().find_map(|k| map.get(*k)).and_then(scalar) {
        out.push_str(&level_token(&level));
        out.push(' ');
    }
    if let Some(msg) = MSG.iter().find_map(|k| map.get(*k)).and_then(scalar) {
        out.push_str(&bold(&msg));
        out.push(' ');
    }
    for (k, v) in map {
        if LEAD.contains(&k.as_str()) || MSG.contains(&k.as_str()) {
            continue;
        }
        let Some(val) = scalar(v) else {
            out.push_str(&format!("{}{} ", c(k, p().cyan), dim(&format!("={v}"))));
            continue;
        };
        out.push_str(&format!("{}{}{} ", c(k, p().cyan), dim("="), c(&val, p().fg)));
    }
    out.trim_end().to_string()
}

fn scalar(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn highlight_text(body: &str) -> String {
    let mut out = Vec::new();
    for word in body.split_inclusive(' ') {
        let bare = word.trim().trim_matches(|c: char| "[]():,\"".contains(c));
        if level_color(bare).is_some() {
            out.push(word.replace(bare, &level_token(bare)));
        } else if let Ok(code) = bare.parse::<u16>()
            && (100..600).contains(&code)
            && body.contains("HTTP")
        {
            let col = match code {
                200..=299 => p().green,
                300..=399 => p().cyan,
                400..=499 => p().yellow,
                _ => p().red,
            };
            out.push(word.replace(bare, &c(bare, col)));
        } else {
            out.push(word.to_string());
        }
    }
    out.concat()
}

fn level_color(word: &str) -> Option<Rgb> {
    let w = word.to_ascii_uppercase();
    match w.as_str() {
        "ERROR" | "ERRO" | "ERR" | "FATAL" | "PANIC" | "CRITICAL" | "CRIT" => Some(p().red),
        "WARN" | "WARNING" | "WARM" => Some(p().yellow),
        "INFO" | "INF" | "NOTICE" | "LOG" => Some(p().green),
        "DEBUG" | "DBG" | "TRACE" => Some(p().gray),
        _ => None,
    }
}

fn level_token(word: &str) -> String {
    match level_color(word) {
        Some(col) => cb(word, col),
        None => word.to_string(),
    }
}
