//! `dok events` — the daemon event stream, colour-coded and legible.

use anyhow::Result;
use bollard::models::EventMessage;
use bollard::query_parameters::EventsOptionsBuilder;
use chrono::{Local, TimeZone};
use futures_util::StreamExt;
use std::collections::HashMap;

use crate::dk;
use crate::fmt;
use crate::theme::{self, *};

pub async fn run(
    since: Option<String>,
    until: Option<String>,
    types: Vec<String>,
    filter: Option<String>,
    with_exec: bool,
) -> Result<()> {
    let docker = dk::connect()?;

    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    if !types.is_empty() {
        filters.insert("type".into(), types.clone());
    }

    let since = since.map(|s| to_timestamp(&s));
    let until = until.map(|u| to_timestamp(&u));

    let mut builder = EventsOptionsBuilder::default().filters(&filters);
    if let Some(s) = &since {
        builder = builder.since(s);
    }
    if let Some(u) = &until {
        builder = builder.until(u);
    }
    let mut stream = docker.events(Some(builder.build()));

    println!("{}", dim("watching docker events"));

    // Demo mode replays a canned minute of the example stack and exits, so the
    // docs can show the stream without a daemon (and without waiting for one).
    if crate::demo::enabled() {
        for ev in crate::demo::events() {
            let action = ev.action.clone().unwrap_or_default();
            if !with_exec && action.starts_with("exec_") {
                continue;
            }
            let line = render(&ev);
            if let Some(f) = &filter
                && !strip_ansi(&line).to_lowercase().contains(&f.to_lowercase())
            {
                continue;
            }
            println!("{line}");
        }
        return Ok(());
    }

    while let Some(item) = stream.next().await {
        let ev = match item {
            Ok(e) => e,
            Err(e) => {
                eprintln!("{} {}", c("!", p().red), dim(&e.to_string()));
                break;
            }
        };
        let action = ev.action.clone().unwrap_or_default();
        // exec_* fires for every `docker exec` and healthcheck probe; it drowns
        // out the lifecycle events people actually watch for.
        if !with_exec && action.starts_with("exec_") {
            continue;
        }
        let line = render(&ev);
        if let Some(f) = &filter
            && !strip_ansi(&line).to_lowercase().contains(&f.to_lowercase())
        {
            continue;
        }
        println!("{line}");
    }
    Ok(())
}

fn render(ev: &EventMessage) -> String {
    let kind = ev.typ.map(|t| t.to_string()).unwrap_or_default();
    let action = ev.action.clone().unwrap_or_default();
    let attrs = ev.actor.as_ref().and_then(|a| a.attributes.clone()).unwrap_or_default();

    let when = ev
        .time
        .and_then(|t| Local.timestamp_opt(t, 0).single())
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".into());

    // Subject: the container/image/volume name, falling back to the actor id.
    let subject = attrs
        .get("name")
        .or_else(|| attrs.get("image"))
        .cloned()
        .or_else(|| ev.actor.as_ref().and_then(|a| a.id.clone()).map(|i| shorten_id(&i)))
        .unwrap_or_default();

    // health_status events carry their verdict after a colon.
    let (action_word, detail) = match action.split_once(": ") {
        Some((a, d)) => (a.to_string(), d.to_string()),
        None => (action.clone(), String::new()),
    };

    let acol = action_color(&action_word, &detail);
    let mut line = format!(
        "{} {} {} {}",
        dim(&when),
        c(&fmt::pad(&kind, 9), kind_color(&kind)),
        cb(&fmt::pad(&action_word, 14), acol),
        c(&subject, theme::hash_color(&subject))
    );
    if !detail.is_empty() {
        line.push(' ');
        line.push_str(&c(&fmt::truncate(&detail, 60), acol));
    }

    // A couple of attributes worth surfacing inline; the rest stay hidden.
    let mut extras = Vec::new();
    if kind == "container"
        && let Some(img) = attrs.get("image")
        && Some(img) != attrs.get("name")
    {
        extras.push(format!("{}{}", dim("image="), dim(img)));
    }
    if let Some(code) = attrs.get("exitCode") {
        let col = if code == "0" { p().green } else { p().red };
        extras.push(format!("{}{}", dim("exit="), c(code, col)));
    }
    if let Some(sig) = attrs.get("signal") {
        extras.push(format!("{}{}", dim("signal="), c(sig, p().yellow)));
    }
    if !extras.is_empty() {
        line.push_str("  ");
        line.push_str(&extras.join(" "));
    }
    line
}

/// Only 64-char hex ids get truncated; volume and network actors carry their
/// name in the id field and must stay intact.
fn shorten_id(id: &str) -> String {
    let hex = id.len() >= 32 && id.chars().all(|c| c.is_ascii_hexdigit());
    if hex { fmt::short_id(id) } else { id.to_string() }
}

fn kind_color(kind: &str) -> Rgb {
    match kind {
        "container" => p().blue,
        "image" => p().magenta,
        "volume" => p().cyan,
        "network" => p().cyan,
        "builder" => p().orange,
        "daemon" => p().yellow,
        _ => p().gray,
    }
}

fn action_color(action: &str, detail: &str) -> Rgb {
    match action {
        "start" | "create" | "pull" | "connect" | "restart" | "unpause" => p().green,
        "die" | "kill" | "oom" | "destroy" | "delete" | "remove" | "disconnect" | "fail" => p().red,
        "stop" | "pause" | "prune" | "untag" => p().yellow,
        "health_status" => match detail {
            "healthy" => p().green,
            "unhealthy" => p().red,
            _ => p().yellow,
        },
        _ => p().gray,
    }
}

/// Accept `30m` / `2h` / `7d` as well as the timestamps the API expects.
fn to_timestamp(input: &str) -> String {
    let (num, unit) = input.split_at(input.len().saturating_sub(1));
    let Ok(n) = num.parse::<i64>() else { return input.to_string() };
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        "w" => n * 604_800,
        _ => return input.to_string(),
    };
    (chrono::Utc::now().timestamp() - secs).to_string()
}

/// Filtering happens on the rendered line, so drop the colour first.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}
