//! `dok compose` — compose projects as declared against as running: missing
//! services, orphans, replicas short of the count, one-off `run` containers.

use anyhow::{Result, bail};
use bollard::models::ContainerSummary;
use serde_json::{Value, json};

use crate::cmds::ps;
use crate::compose::{self, Project, Service, State, Unchecked};
use crate::dk;
use crate::fmt;
use crate::json;
use crate::table::{Column, Table};
use crate::theme::{self, *};

pub async fn run(project: Option<String>) -> Result<()> {
    let docker = dk::connect()?;
    // Stopped containers are half the story: a service that exited is down,
    // not absent.
    let containers = dk::containers(&docker, true).await?;
    let groups = compose::group(&containers);

    let Some(wanted) = project else {
        let projects: Vec<Project> = groups
            .iter()
            .map(|(name, cs)| compose::build(name, cs, compose::declared_for(name, cs)))
            .collect();
        return list(&projects);
    };

    let name = pick(&wanted, groups.keys().map(String::as_str).collect())?;
    let members = &groups[name];
    let project = compose::build(name, members, compose::declared_for(name, members));
    detail(&project)
}

/// An exact name, else the one project the needle is a prefix or part of.
fn pick<'a>(needle: &str, names: Vec<&'a str>) -> Result<&'a str> {
    if let Some(n) = names.iter().find(|n| **n == needle) {
        return Ok(n);
    }
    let close: Vec<&str> = {
        let prefix: Vec<&str> = names.iter().copied().filter(|n| n.starts_with(needle)).collect();
        if prefix.is_empty() {
            names.iter().copied().filter(|n| n.contains(needle)).collect()
        } else {
            prefix
        }
    };
    match close.as_slice() {
        [one] => Ok(one),
        [] if names.is_empty() => {
            bail!("no compose projects (no container carries compose labels)")
        }
        [] => bail!("no compose project matches `{needle}`; projects: {}", names.join(", ")),
        many => bail!("`{needle}` matches several projects: {}", many.join(", ")),
    }
}

// ── every project ───────────────────────────────────────────────────────────

fn list(projects: &[Project]) -> Result<()> {
    if json::enabled() {
        json::emit(&json!({ "projects": projects.iter().map(summary_json).collect::<Vec<_>>() }));
        return Ok(());
    }
    if projects.is_empty() {
        println!("{}", dim("no compose projects"));
        return Ok(());
    }

    let mut t = Table::new(vec![
        Column::left(""),
        Column::left("PROJECT").flex(10).cap(30),
        Column::right("SERVICES"),
        Column::right("CONTAINERS"),
        Column::left("ISSUES").flex(10).cap(40),
        Column::left("DIRECTORY").flex(10).cap(40),
    ]);
    for proj in projects {
        let running: usize = proj.services.iter().map(Service::running).sum();
        let total: usize = proj.services.iter().map(|s| s.containers.len()).sum();
        let (col, glyph) = project_style(proj);
        t.row(vec![
            c(glyph, col),
            cb(&proj.name, p().blue),
            ratio(proj.up(), proj.wanted()),
            ratio(running, total),
            issues_cell(proj),
            dim(&home(proj.working_dir.as_deref().unwrap_or(""))),
        ]);
    }
    t.print();

    let bad = projects.iter().filter(|proj| !proj.healthy()).count();
    let line = if bad == 0 {
        c(&format!("{} project{} as declared", projects.len(), plural(projects.len())), p().green)
    } else {
        c(
            &format!(
                "{bad} of {} project{} need attention",
                projects.len(),
                plural(projects.len())
            ),
            p().yellow,
        )
    };
    println!("\n{line}");
    if projects.iter().any(|proj| proj.unchecked.is_some()) {
        println!(
            "{}",
            dim(
                "missing and orphaned services are only checked against compose files dok can read"
            )
        );
    }
    Ok(())
}

fn project_style(proj: &Project) -> (Rgb, &'static str) {
    let issues = proj.issues();
    if proj.services.iter().all(|s| s.running() == 0) {
        theme::state_style("exited")
    } else if issues.contains_key(&State::Missing) || issues.contains_key(&State::Down) {
        (p().red, theme::icon(g().fail, "\u{f057}"))
    } else if !proj.healthy() {
        (p().yellow, theme::icon(g().pending, "\u{f071}"))
    } else {
        theme::state_style("running")
    }
}

/// `1 down · 1 missing · 1 unhealthy`, worst first.
fn issues_cell(proj: &Project) -> String {
    let mut parts: Vec<String> = proj
        .issues()
        .iter()
        .map(|(state, n)| c(&format!("{n} {}", state.as_str()), state_color(*state)))
        .collect();
    let unhealthy = proj.unhealthy();
    if unhealthy > 0 {
        parts.push(c(&format!("{unhealthy} unhealthy"), p().red));
    }
    if parts.is_empty() { dim("—") } else { parts.join(&dim(" · ")) }
}

// ── one project ─────────────────────────────────────────────────────────────

fn detail(proj: &Project) -> Result<()> {
    if json::enabled() {
        let mut v = summary_json(proj);
        v["services"] = proj.services.iter().map(service_json).collect();
        v["oneoffs"] = proj.oneoffs.iter().map(|c| container_json(c)).collect();
        json::emit(&v);
        return Ok(());
    }

    let (col, glyph) = project_style(proj);
    let mut head = format!(
        "{} {}  {} {}",
        c(glyph, col),
        cb(&proj.name, p().blue),
        dim("·"),
        ratio(proj.up(), proj.wanted()),
    );
    head.push_str(&dim(" services up"));
    if let Some(files) = &proj.config_files {
        let files: Vec<String> = files.split(',').map(home).collect();
        head.push_str(&format!("  {} {}", dim("·"), dim(&files.join(", "))));
    }
    println!("{head}\n");

    let mut t = Table::new(vec![
        Column::left(""),
        Column::left("SERVICE").flex(10).cap(30),
        Column::right("REPLICAS"),
        Column::left("IMAGE").flex(14).cap(32),
        Column::left("STATUS"),
        Column::left("PORTS").flex(0).cap(22),
    ]);
    for s in &proj.services {
        service_rows(&mut t, s);
    }
    if !proj.oneoffs.is_empty() {
        t.blank();
        t.group(format!(
            "{} {}  {}",
            c(theme::icon(g().group_mark, "\u{f0e8}"), p().gray),
            cb("one-off", p().gray),
            dim("docker compose run")
        ));
        let n = proj.oneoffs.len();
        for (i, ct) in proj.oneoffs.iter().enumerate() {
            let stub = if i + 1 == n { g().tree_last } else { g().tree_branch };
            let svc = dk::label(ct, dk::COMPOSE_SERVICE).unwrap_or("");
            let (scol, sglyph) = theme::state_style(&dk::state_of(ct));
            t.row(vec![
                c(sglyph, scol),
                format!("{}{} {}", dim(stub), c(svc, p().fg), dim(&dk::name_of(ct))),
                String::new(),
                ps::image_cell(ct.image.as_deref().unwrap_or("")),
                ps::status_cell(ct, &dk::state_of(ct), scol),
                ps::ports_cell(ct),
            ]);
        }
    }
    t.print();

    for line in footer(proj) {
        println!("{line}");
    }
    Ok(())
}

fn service_rows(t: &mut Table, s: &Service) {
    let col = state_color(s.state);
    let glyph = match s.state {
        State::Running => theme::state_style("running").1,
        State::Down => theme::state_style("exited").1,
        State::Missing => theme::icon(g().fail, "\u{f057}"),
        State::Degraded | State::Orphan => theme::icon(g().pending, "\u{f071}"),
        State::Inactive => theme::state_style("created").1,
    };

    let mut name = cb(&s.name, if s.running() > 0 { p().fg } else { p().gray });
    // One verdict for the service: any unhealthy replica is the headline.
    if let Some(h) = service_health(s)
        && let Some((hc, hg)) = theme::health_style(&h)
    {
        name.push(' ');
        name.push_str(&c(hg, hc));
    }

    let replicas = match (s.state, s.expected) {
        (State::Inactive, _) => dim("—"),
        (State::Orphan, _) => c(&s.running().to_string(), col),
        (_, Some(e)) => ratio(s.running(), e as usize),
        (_, None) => c(&format!("{}/?", s.running()), p().fg),
    };

    let one = s.containers.len() == 1;
    let status = match s.state {
        State::Missing => c("missing", col),
        State::Orphan => c("orphan", col),
        State::Inactive => dim(&format!("profile {}", s.profiles.join(", "))),
        _ if one => {
            let ct = s.containers[0];
            let st = dk::state_of(ct);
            ps::status_cell(ct, &st, theme::state_style(&st).0)
        }
        State::Degraded => c("degraded", col),
        State::Down => c("down", col),
        State::Running => c("up", col),
    };

    let ports = if one { ps::ports_cell(s.containers[0]) } else { String::new() };
    t.row(vec![
        c(glyph, col),
        name,
        replicas,
        if s.image.is_empty() { dim("—") } else { ps::image_cell(&s.image) },
        status,
        ports,
    ]);

    // Several replicas: one line each, so the one that died is visible.
    if s.containers.len() > 1 {
        let n = s.containers.len();
        for (i, ct) in s.containers.iter().enumerate() {
            let st = dk::state_of(ct);
            let (scol, sglyph) = theme::state_style(&st);
            let stub = if i + 1 == n { g().tree_last } else { g().tree_branch };
            let num = dk::label(ct, compose::NUMBER).unwrap_or("?");
            t.row(vec![
                c(sglyph, scol),
                format!("{}{}", dim(stub), dim(&format!("#{num}"))),
                String::new(),
                String::new(),
                ps::status_cell(ct, &st, scol),
                ps::ports_cell(ct),
            ]);
        }
    }
}

/// The worst healthcheck verdict among running replicas.
fn service_health(s: &Service) -> Option<String> {
    let verdicts: Vec<String> = s
        .containers
        .iter()
        .filter(|c| dk::state_of(c) == "running")
        .filter_map(|c| dk::health_of(c))
        .collect();
    ["unhealthy", "starting", "healthy"]
        .into_iter()
        .find(|v| verdicts.iter().any(|h| h == v))
        .map(str::to_string)
}

/// Totals, then what would fix each problem.
fn footer(proj: &Project) -> Vec<String> {
    let mut out = vec![String::new()];
    let issues = proj.issues();
    let unhealthy = proj.unhealthy();
    if issues.is_empty() && unhealthy == 0 {
        out.push(c("as declared", p().green));
    } else {
        out.push(issues_cell(proj));
    }

    let start = [State::Missing, State::Down, State::Degraded];
    if start.iter().any(|s| issues.contains_key(s)) {
        out.push(dim(&format!("start them:      docker compose -p {} up -d", proj.name)));
    }
    if issues.contains_key(&State::Orphan) {
        out.push(dim(&format!(
            "remove orphans:  docker compose -p {} up -d --remove-orphans",
            proj.name
        )));
    }
    if let Some(why) = &proj.unchecked {
        out.push(dim(&format!(
            "{}; missing and orphaned services not checked",
            unchecked_text(why)
        )));
    }
    out
}

fn unchecked_text(u: &Unchecked) -> String {
    match u {
        Unchecked::Remote => "the compose files are on the daemon's host".into(),
        Unchecked::NoFiles => "compose recorded no config files".into(),
        Unchecked::Unreadable(e) => e.clone(),
    }
}

// ── shared bits ─────────────────────────────────────────────────────────────

fn state_color(s: State) -> Rgb {
    match s {
        State::Running => p().green,
        State::Missing | State::Down => p().red,
        State::Degraded | State::Orphan => p().yellow,
        State::Inactive => p().gray,
    }
}

/// `3/4`, green when complete.
fn ratio(have: usize, want: usize) -> String {
    let text = format!("{have}/{want}");
    if have >= want { c(&text, p().green) } else { c(&text, p().yellow) }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `/home/me/src/app` -> `~/src/app`.
fn home(path: &str) -> String {
    let path = path.trim();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && dk::is_local() => match path.strip_prefix(&h) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
            _ => path.to_string(),
        },
        _ => path.to_string(),
    }
}

fn summary_json(proj: &Project) -> Value {
    let issues: serde_json::Map<String, Value> =
        proj.issues().iter().map(|(s, n)| (s.as_str().to_string(), json!(n))).collect();
    json!({
        "name": proj.name,
        "working_dir": proj.working_dir,
        "config_files": proj.config_files.as_deref().map(|f| f.split(',').map(str::trim).collect::<Vec<_>>()),
        "services_up": proj.up(),
        "services_wanted": proj.wanted(),
        "containers_running": proj.services.iter().map(Service::running).sum::<usize>(),
        "containers_total": proj.services.iter().map(|s| s.containers.len()).sum::<usize>(),
        "unhealthy": proj.unhealthy(),
        "issues": issues,
        "as_declared": proj.healthy(),
        "checked_against_files": proj.unchecked.is_none(),
        "unchecked_reason": proj.unchecked.as_ref().map(unchecked_text),
    })
}

fn service_json(s: &Service) -> Value {
    json!({
        "name": s.name,
        "state": s.state.as_str(),
        "expected": s.expected,
        "running": s.running(),
        "image": json::opt(&s.image),
        "profiles": s.profiles,
        "health": service_health(s),
        "containers": s.containers.iter().map(|c| container_json(c)).collect::<Vec<_>>(),
    })
}

fn container_json(c: &ContainerSummary) -> Value {
    json!({
        "id": c.id.as_deref().unwrap_or(""),
        "name": fmt::short_task_name(&dk::name_of(c)),
        "number": dk::label(c, compose::NUMBER).and_then(|n| n.parse::<u32>().ok()),
        "service": json::opt(dk::label(c, dk::COMPOSE_SERVICE).unwrap_or("")),
        "state": dk::state_of(c),
        "status": c.status.clone().unwrap_or_default(),
        "health": dk::health_of(c),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_picked_by_name_prefix_or_part() {
        let names = vec!["demo-shop", "observability", "shop-admin"];
        assert_eq!(pick("demo-shop", names.clone()).unwrap(), "demo-shop");
        assert_eq!(pick("obs", names.clone()).unwrap(), "observability");
        assert_eq!(pick("admin", names.clone()).unwrap(), "shop-admin");
        // A prefix beats a match further in: `shop` is shop-admin, not demo-shop.
        assert_eq!(pick("shop", names.clone()).unwrap(), "shop-admin");
        let err = pick("shop", vec!["shop-a", "shop-b"]).unwrap_err().to_string();
        assert!(err.contains("several"), "{err}");
        let err = pick("nope", names).unwrap_err().to_string();
        assert!(err.contains("projects: demo-shop, observability, shop-admin"), "{err}");
        assert!(pick("x", vec![]).unwrap_err().to_string().contains("no compose projects"));
    }
}
