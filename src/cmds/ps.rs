//! `dok ps` — containers as a coloured, compose-grouped table.

use anyhow::Result;
use bollard::models::ContainerSummary;
use clap::ValueEnum;
use std::collections::BTreeMap;

use crate::dk;
use crate::fmt;
use crate::table::{Column, Table};
use crate::theme::{self, *};

#[derive(Copy, Clone, ValueEnum)]
pub enum PsSort {
    Name,
    Age,
    State,
    Image,
}

pub async fn run(all: bool, flat: bool, filter: Option<String>, sort: PsSort) -> Result<()> {
    let docker = dk::connect()?;
    let mut list = dk::containers(&docker, all).await?;

    if let Some(f) = &filter {
        let f = f.to_lowercase();
        list.retain(|c| {
            dk::name_of(c).to_lowercase().contains(&f)
                || c.image.as_deref().unwrap_or("").to_lowercase().contains(&f)
        });
    }

    if list.is_empty() {
        println!("{}", dim(if all { "no containers" } else { "no running containers (try -a)" }));
        return Ok(());
    }

    sort_containers(&mut list, sort);

    let mut t = Table::new(vec![
        Column::left(""),
        Column::left("ID"),
        Column::left("NAME").flex(12).cap(30),
        Column::left("IMAGE").flex(14).cap(32),
        Column::left("STATUS"),
        Column::left("PORTS").flex(0).cap(22),
        Column::right("AGE"),
    ]);

    if flat {
        for c in &list {
            t.row(render(c, Stub::None));
        }
    } else {
        // Group by compose project; loose containers land in a trailing group.
        let mut groups: BTreeMap<String, Vec<&ContainerSummary>> = BTreeMap::new();
        for c in &list {
            let key = dk::label(c, dk::COMPOSE_PROJECT).unwrap_or("").to_string();
            groups.entry(key).or_default().push(c);
        }
        let mut first = true;
        // Named projects first, standalone containers last.
        let mut keys: Vec<String> = groups.keys().cloned().collect();
        keys.sort_by_key(|k| (k.is_empty(), k.clone()));

        for key in keys {
            let members = &groups[&key];
            if !first {
                t.blank();
            }
            first = false;
            t.group(group_header(&key, members));
            let grouped = !key.is_empty();
            for (i, ct) in members.iter().enumerate() {
                let stub = if !grouped {
                    Stub::None
                } else if i + 1 == members.len() {
                    Stub::Last
                } else {
                    Stub::Mid
                };
                t.row(render(ct, stub));
            }
        }
    }

    t.print();
    println!("{}", summary_line(&list));
    Ok(())
}

fn sort_containers(list: &mut [ContainerSummary], sort: PsSort) {
    match sort {
        PsSort::Name => list.sort_by_key(dk::name_of),
        PsSort::Age => list.sort_by_key(|c| -c.created.unwrap_or(0)),
        PsSort::Image => list.sort_by(|a, b| a.image.cmp(&b.image)),
        PsSort::State => list.sort_by_key(|c| {
            let rank = match dk::state_of(c).as_str() {
                "running" => 0,
                "restarting" => 1,
                "paused" => 2,
                "created" => 3,
                "exited" => 4,
                _ => 5,
            };
            (rank, dk::name_of(c))
        }),
    }
}

fn group_header(project: &str, members: &[&ContainerSummary]) -> String {
    let running = members.iter().filter(|c| dk::state_of(c) == "running").count();
    let icon = theme::icon(g().group_mark, "\u{f0e8}");
    let title = if project.is_empty() { cb("standalone", p().gray) } else { cb(project, p().blue) };
    let counts = format!("{running}/{} up", members.len());
    let counts =
        if running == members.len() { c(&counts, p().green) } else { c(&counts, p().yellow) };
    format!("{} {}  {} {}", c(icon, p().gray), title, dim("·"), counts)
}

/// Tree stub drawn before a name inside a compose group.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Stub {
    None,
    Mid,
    Last,
}

/// One container row.
fn render(ct: &ContainerSummary, stub: Stub) -> Vec<String> {
    let state = dk::state_of(ct);
    let (scol, sglyph) = theme::state_style(&state);

    // Name column: optional tree stub, service name when in a project.
    let name = fmt::short_task_name(&dk::name_of(ct));
    let service = dk::label(ct, dk::COMPOSE_SERVICE);
    let shown = service.unwrap_or(&name);
    let mut name_cell = String::new();
    match stub {
        Stub::None => {}
        Stub::Mid => name_cell.push_str(&dim(g().tree_branch)),
        Stub::Last => name_cell.push_str(&dim(g().tree_last)),
    }
    name_cell.push_str(&cb(shown, if state == "running" { p().fg } else { p().gray }));
    // A healthcheck verdict is only meaningful while the container runs.
    if state == "running"
        && let Some(h) = dk::health_of(ct)
        && let Some((hc, hg)) = theme::health_style(&h)
    {
        name_cell.push(' ');
        name_cell.push_str(&c(hg, hc));
    }

    // Image column: dim registry prefix, bright name, coloured tag.
    let image = ct.image.clone().unwrap_or_default();
    let (prefix, iname, tag) = fmt::split_image(&image);
    let icon = theme::image_icon(&image);
    let image_cell = format!(
        "{} {}{}{}",
        c(icon, p().magenta),
        dim(&prefix),
        c(&iname, p().fg),
        c(&format!(":{tag}"), tag_color(&tag))
    );

    vec![
        c(sglyph, scol),
        dim(&fmt::short_id(ct.id.as_deref().unwrap_or(""))),
        name_cell,
        image_cell,
        status_cell(ct, &state, scol),
        ports_cell(ct),
        c(
            &fmt::age(dk::age_secs(ct.created.unwrap_or(0))),
            theme::age_color(dk::age_secs(ct.created.unwrap_or(0))),
        ),
    ]
}

fn tag_color(tag: &str) -> Rgb {
    match tag {
        "latest" => p().yellow,
        t if t.starts_with('@') => p().gray,
        t if t.chars().next().is_some_and(|c| c.is_ascii_digit() || c == 'v') => p().cyan,
        _ => p().magenta,
    }
}

/// `up 3h`, `exited (137)`, `restarting`, … — compacted from docker's prose.
fn status_cell(ct: &ContainerSummary, state: &str, scol: Rgb) -> String {
    let raw = ct.status.clone().unwrap_or_default();
    let text = match state {
        "running" => {
            let up = raw.strip_prefix("Up ").unwrap_or("");
            let up = up.split(" (").next().unwrap_or(up);
            if up.is_empty() { "up".to_string() } else { format!("up {}", compact_duration(up)) }
        }
        "exited" => {
            let code = raw
                .split_once('(')
                .and_then(|(_, r)| r.split_once(')'))
                .map(|(c, _)| c.to_string())
                .unwrap_or_default();
            if code.is_empty() { "exited".into() } else { format!("exited ({code})") }
        }
        "" => raw,
        s => s.to_string(),
    };
    // A non-zero exit is worth shouting about even though "exited" is neutral.
    let col = if text.starts_with("exited (") && !text.contains("(0)") { p().red } else { scol };
    c(&text, col)
}

/// "3 hours" -> "3h", "About a minute" -> "1m".
fn compact_duration(s: &str) -> String {
    let mut it = s.split_whitespace();
    let (Some(n), Some(unit)) = (it.next(), it.next()) else {
        return s.to_string();
    };
    let n = if n.eq_ignore_ascii_case("about") || n.eq_ignore_ascii_case("a") { "1" } else { n };
    let u = match unit.trim_end_matches('s') {
        "second" => "s",
        "minute" => "m",
        "hour" => "h",
        "day" => "d",
        "week" => "w",
        "month" => "mo",
        "year" => "y",
        _ => return s.to_string(),
    };
    // "About a minute" reaches here as ("About", "a") — recover the unit.
    if n == "1" && u == "s" && unit == "a" {
        return "1m".into();
    }
    format!("{n}{u}")
}

/// `:8080→80  :443→443/tcp` with unpublished ports dimmed away entirely.
fn ports_cell(ct: &ContainerSummary) -> String {
    let Some(ports) = &ct.ports else { return String::new() };
    let mut seen = Vec::new();
    for p in ports {
        let Some(public) = p.public_port else { continue };
        let proto = p.typ.map(|t| t.to_string()).unwrap_or_default();
        let entry = if public == p.private_port {
            format!(":{public}")
        } else {
            format!(":{public}{}{}", g().arrow, p.private_port)
        };
        let entry = if proto == "udp" { format!("{entry}/udp") } else { entry };
        if !seen.contains(&entry) {
            seen.push(entry);
        }
    }
    if seen.is_empty() {
        // Exposed-but-unpublished ports: show them, quietly.
        let mut inner: Vec<String> =
            ports.iter().map(|p| p.private_port.to_string()).collect::<Vec<_>>();
        inner.sort();
        inner.dedup();
        return dim(&inner.join(" "));
    }
    seen.iter().map(|s| c(s, p().cyan)).collect::<Vec<_>>().join(" ")
}

fn summary_line(list: &[ContainerSummary]) -> String {
    let running = list.iter().filter(|c| dk::state_of(c) == "running").count();
    let stopped = list.len() - running;
    let unhealthy = list
        .iter()
        .filter(|c| {
            dk::state_of(c) == "running" && dk::health_of(c).as_deref() == Some("unhealthy")
        })
        .count();
    let mut parts = vec![c(&format!("{running} running"), p().green)];
    if stopped > 0 {
        parts.push(c(&format!("{stopped} stopped"), p().gray));
    }
    if unhealthy > 0 {
        parts.push(c(&format!("{unhealthy} unhealthy"), p().red));
    }
    format!("\n{}", parts.join(&dim(" · ")))
}
