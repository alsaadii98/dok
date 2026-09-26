//! `dok health` — only the containers that have a healthcheck.
//!
//! `docker ps` shows `(healthy)` buried in a status string and nothing else.
//! The three things worth knowing when a check is failing — how long it has
//! been failing, what the probe actually printed, and how often it runs — live
//! in `docker inspect`, several hundred lines apart.

use anyhow::Result;
use bollard::models::ContainerInspectResponse;
use bollard::query_parameters::InspectContainerOptions;

use crate::dk;
use crate::fmt;
use crate::json;
use crate::table::{Column, Table};
use crate::theme::{self, *};

struct Check {
    name: String,
    project: String,
    status: String,
    streak: i64,
    /// First line of the last probe's output.
    probe: String,
    exit_code: i64,
    /// Healthcheck interval in seconds, if the image declares one.
    interval: Option<i64>,
}

pub async fn run(all: bool, failing_only: bool) -> Result<()> {
    let docker = dk::connect()?;
    let list = dk::containers(&docker, all).await?;
    let total = list.len();

    let mut checks = Vec::new();
    for ct in &list {
        let name = dk::name_of(ct);
        let detail = if crate::demo::enabled() {
            crate::demo::inspect(&name)
        } else {
            docker.inspect_container(&name, None::<InspectContainerOptions>).await?
        };
        if let Some(check) = extract(&detail, ct) {
            checks.push(check);
        }
    }

    // Counted before filtering: with --unhealthy the containers that pass are
    // still containers that have a healthcheck, and must not be reported as
    // lacking one.
    let with_checks = checks.len();
    if failing_only {
        checks.retain(|c| c.status == "unhealthy" || c.streak > 0);
    }

    if json::enabled() {
        checks.sort_by(|a, b| {
            rank(&a.status).cmp(&rank(&b.status)).then_with(|| a.name.cmp(&b.name))
        });
        json::emit(&serde_json::json!({
            "checks": checks.iter().map(|k| serde_json::json!({
                "container": k.name,
                "project": json::opt(&k.project),
                "status": k.status,
                "failing_streak": k.streak,
                "interval_seconds": k.interval,
                "last_probe": serde_json::json!({
                    "exit_code": k.exit_code,
                    "output": json::opt(&k.probe),
                }),
            })).collect::<Vec<_>>(),
            "summary": {
                "with_healthcheck": with_checks,
                "without_healthcheck": total.saturating_sub(with_checks),
                "containers": total,
            },
        }));
        return Ok(());
    }

    if checks.is_empty() {
        let msg = if failing_only {
            "no failing healthchecks"
        } else if total == 0 {
            "no containers"
        } else {
            "no containers declare a healthcheck"
        };
        println!("{}", dim(msg));
        return Ok(());
    }

    // Unhealthy first: this command exists to be read when something is wrong.
    checks.sort_by(|a, b| rank(&a.status).cmp(&rank(&b.status)).then_with(|| a.name.cmp(&b.name)));

    let mut t = Table::new(vec![
        Column::left(""),
        Column::left("CONTAINER").flex(10).cap(26),
        Column::left("PROJECT").flex(8).cap(22),
        Column::left("HEALTH"),
        Column::right("FAILING"),
        Column::right("EVERY"),
        Column::left("LAST PROBE").flex(16).cap(40),
    ]);
    for c in &checks {
        t.row(render(c));
    }
    t.print();
    println!("{}", summary(&checks, total, with_checks));
    Ok(())
}

/// Unhealthy sorts first, then starting, then healthy.
fn rank(status: &str) -> u8 {
    match status {
        "unhealthy" => 0,
        "starting" => 1,
        "healthy" => 2,
        _ => 3,
    }
}

fn extract(d: &ContainerInspectResponse, ct: &bollard::models::ContainerSummary) -> Option<Check> {
    let state = d.state.as_ref()?;
    // A container with no healthcheck has no business in this table, even
    // though the daemon reports a "none" status for it.
    let health = state.health.as_ref()?;
    let status = health.status.map(|s| s.to_string()).unwrap_or_default();
    if status.is_empty() || status == "none" {
        return None;
    }

    let last = health.log.as_ref().and_then(|l| l.last());
    let probe = last
        .and_then(|r| r.output.clone())
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    let interval = d
        .config
        .as_ref()
        .and_then(|c| c.healthcheck.as_ref())
        .and_then(|h| h.interval)
        // Docker stores healthcheck durations in nanoseconds.
        .map(|n| n / 1_000_000_000);

    Some(Check {
        name: dk::label(ct, dk::COMPOSE_SERVICE)
            .map(str::to_string)
            .unwrap_or_else(|| fmt::short_task_name(&dk::name_of(ct))),
        project: dk::label(ct, dk::COMPOSE_PROJECT).unwrap_or("").to_string(),
        status,
        streak: health.failing_streak.unwrap_or(0),
        probe,
        exit_code: last.and_then(|r| r.exit_code).unwrap_or(0),
        interval,
    })
}

fn render(k: &Check) -> Vec<String> {
    let (col, glyph) = theme::health_style(&k.status).unwrap_or((p().gray, g().bullet));

    // A streak of zero is the normal case and should not shout.
    let streak = if k.streak > 0 {
        c(&k.streak.to_string(), if k.streak > 3 { p().red } else { p().orange })
    } else {
        dim("0")
    };

    let every = match k.interval {
        Some(s) if s > 0 => dim(&fmt::age(s)),
        _ => dim("—"),
    };

    // A non-zero exit is the reason the probe failed, and it is not otherwise
    // visible anywhere in dok's output.
    let probe = if k.probe.is_empty() {
        dim("—")
    } else if k.exit_code != 0 {
        format!("{} {}", c(&format!("exit {}", k.exit_code), p().red), dim(&k.probe))
    } else {
        dim(&k.probe)
    };

    vec![
        c(glyph, col),
        cb(&k.name, if k.status == "unhealthy" { p().red } else { p().fg }),
        if k.project.is_empty() { String::new() } else { c(&k.project, p().blue) },
        c(&k.status, col),
        streak,
        every,
        probe,
    ]
}

fn summary(checks: &[Check], total: usize, with_checks: usize) -> String {
    let n = |s: &str| checks.iter().filter(|c| c.status == s).count();
    let (healthy, unhealthy, starting) = (n("healthy"), n("unhealthy"), n("starting"));

    let mut parts = Vec::new();
    if healthy > 0 {
        parts.push(c(&format!("{healthy} healthy"), p().green));
    }
    if starting > 0 {
        parts.push(c(&format!("{starting} starting"), p().yellow));
    }
    if unhealthy > 0 {
        parts.push(c(&format!("{unhealthy} unhealthy"), p().red));
    }
    // The containers with no healthcheck at all are worth a number: an empty
    // table and a table missing half the stack look the same otherwise.
    let without = total.saturating_sub(with_checks);
    if without > 0 {
        parts.push(dim(&format!("{without} without a healthcheck")));
    }
    format!("\n{}", parts.join(&dim(" · ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unhealthy_sorts_above_everything_else() {
        let mut v = ["healthy", "unhealthy", "starting", "none"];
        v.sort_by_key(|s| rank(s));
        assert_eq!(v, ["unhealthy", "starting", "healthy", "none"]);
    }
}
