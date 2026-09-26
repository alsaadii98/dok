//! `dok inspect` — the 400-line JSON folded into something you can read.

use anyhow::Result;
use bollard::models::ContainerInspectResponse;
use bollard::query_parameters::InspectContainerOptions;

use crate::dk;
use crate::fmt;
use crate::json;
use crate::theme::{self, *};

/// Env keys whose values are hidden unless `--show-secrets` is passed.
const SECRETISH: [&str; 8] =
    ["PASSWORD", "PASSWD", "SECRET", "TOKEN", "APIKEY", "API_KEY", "PRIVATE", "CREDENTIAL"];

pub async fn run(wanted: Vec<String>, show_secrets: bool, show_env: bool) -> Result<()> {
    let docker = dk::connect()?;
    let mut docs = Vec::new();
    for (i, w) in wanted.iter().enumerate() {
        if i > 0 && !json::enabled() {
            println!();
        }
        let ct = if crate::demo::enabled() {
            crate::demo::inspect(w)
        } else {
            let name = dk::resolve(&docker, w).await?;
            docker.inspect_container(&name, None::<InspectContainerOptions>).await?
        };
        if json::enabled() {
            docs.push(inspect_json(&ct, show_secrets, show_env));
        } else {
            render(&ct, show_secrets, show_env);
        }
    }
    if json::enabled() {
        json::emit(&serde_json::json!({ "containers": docs }));
    }
    Ok(())
}

/// The folded view as an object.
///
/// Masking is the table's, not relaxed: env is absent without `--env`, and a
/// credential-looking key is replaced by its length unless `--show-secrets`
/// was passed. `--json` must not be a way to read what the table hides.
fn inspect_json(
    ct: &ContainerInspectResponse,
    show_secrets: bool,
    show_env: bool,
) -> serde_json::Value {
    let state = ct.state.as_ref();
    let cfg = ct.config.as_ref();
    let health = state.and_then(|s| s.health.as_ref());

    let env = if show_env {
        cfg.and_then(|c| c.env.as_ref())
            .map(|env| {
                env.iter()
                    .map(|e| {
                        let (k, v) = e.split_once('=').unwrap_or((e.as_str(), ""));
                        let upper = k.to_ascii_uppercase();
                        let hidden = !show_secrets && SECRETISH.iter().any(|s| upper.contains(s));
                        let val = if hidden {
                            serde_json::json!({ "masked": true, "length": v.len() })
                        } else {
                            serde_json::Value::from(v)
                        };
                        (k.to_string(), val)
                    })
                    .collect::<serde_json::Map<_, _>>()
            })
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };

    serde_json::json!({
        "id": ct.id,
        "name": ct.name.clone().unwrap_or_default().trim_start_matches('/'),
        "image": ct.image,
        "platform": ct.platform,
        "state": {
            "status": state.and_then(|s| s.status).map(|s| s.to_string()),
            "running": state.and_then(|s| s.running),
            "pid": state.and_then(|s| s.pid),
            "exit_code": state.and_then(|s| s.exit_code),
            "started_at": state.and_then(|s| s.started_at.clone()),
            "restart_count": ct.restart_count,
            "oom_killed": state.and_then(|s| s.oom_killed),
        },
        "health": health.map(|h| serde_json::json!({
            "status": h.status.map(|s| s.to_string()),
            "failing_streak": h.failing_streak,
            "last_probe": h.log.as_ref().and_then(|l| l.last()).map(|r| serde_json::json!({
                "exit_code": r.exit_code,
                "output": r.output,
            })),
        })),
        "config": {
            "entrypoint": cfg.and_then(|c| c.entrypoint.clone()),
            "command": cfg.and_then(|c| c.cmd.clone()),
            "working_dir": cfg.and_then(|c| c.working_dir.clone()).filter(|w| !w.is_empty()),
            "user": cfg.and_then(|c| c.user.clone()).filter(|u| !u.is_empty()),
            "stop_signal": cfg.and_then(|c| c.stop_signal.clone()),
        },
        // Absent without --env, masked without --show-secrets.
        "env": env,
        "mounts": ct.mounts.as_deref().unwrap_or(&[]).iter().map(|m| serde_json::json!({
            "type": m.typ.as_ref().map(|t| t.to_string()),
            "name": m.name,
            "source": m.source,
            "destination": m.destination,
            "read_write": m.rw,
        })).collect::<Vec<_>>(),
        "labels": cfg.and_then(|c| c.labels.clone()),
    })
}

// ── layout helpers ──────────────────────────────────────────────────────────

const KEY_W: usize = 14;

fn section(title: &str, col: Rgb) {
    println!("\n{} {}", c(theme::icon(g().group_mark, "\u{f0da}"), col), cb(title, col));
}

fn field(key: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    println!("  {} {}", dim(&fmt::pad(key, KEY_W)), value);
}

fn list_field(key: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    for (i, v) in values.iter().enumerate() {
        let k = if i == 0 { key } else { "" };
        println!("  {} {}", dim(&fmt::pad(k, KEY_W)), v);
    }
}

fn yes_no(v: bool, on: Rgb, off: Rgb) -> String {
    if v { c("yes", on) } else { c("no", off) }
}

// ── rendering ───────────────────────────────────────────────────────────────

fn render(ct: &ContainerInspectResponse, show_secrets: bool, show_env: bool) {
    let name = ct.name.clone().unwrap_or_default().trim_start_matches('/').to_string();
    let state = ct.state.as_ref();
    let status = state.and_then(|s| s.status).map(|s| s.to_string()).unwrap_or_default();
    let (scol, sglyph) = theme::state_style(&status);

    println!("{} {}", c(sglyph, scol), cb(&name, scol));

    identity(ct);
    state_section(ct, scol, &status);
    config(ct, show_secrets, show_env);
    resources(ct);
    network(ct);
    mounts(ct);
    labels(ct);
}

fn identity(ct: &ContainerInspectResponse) {
    section("identity", p().blue);
    field("id", &dim(&fmt::short_id(ct.id.as_deref().unwrap_or(""))));
    let image = ct.config.as_ref().and_then(|c| c.image.clone()).unwrap_or_default();
    let (prefix, iname, tag) = fmt::split_image(&image);
    field(
        "image",
        &format!("{}{}{}", dim(&prefix), c(&iname, p().fg), c(&format!(":{tag}"), p().cyan)),
    );
    field("image id", &dim(&fmt::short_id(ct.image.as_deref().unwrap_or(""))));
    if let Some(p) = &ct.platform {
        field("platform", &dim(p));
    }
    if let Some(created) = &ct.created {
        field("created", &dim(&created.to_string()).to_string());
    }
    let size_rw = ct.size_rw.unwrap_or(0);
    if size_rw > 0 {
        field("writable", &c(&fmt::bytes(size_rw as u64), theme::size_color(size_rw as u64)));
    }
}

fn state_section(ct: &ContainerInspectResponse, scol: Rgb, status: &str) {
    let Some(st) = &ct.state else { return };
    section("state", scol);
    field("status", &c(status, scol));
    if let Some(pid) = st.pid.filter(|p| *p > 0) {
        field("pid", &c(&pid.to_string(), p().fg));
    }
    if let Some(started) = &st.started_at {
        field("started", &dim(started));
    }
    if st.running != Some(true)
        && let Some(finished) = &st.finished_at
    {
        field("finished", &dim(finished));
    }
    if let Some(code) = st.exit_code.filter(|_| st.running != Some(true)) {
        field("exit code", &c(&code.to_string(), if code == 0 { p().green } else { p().red }));
    }
    if st.oom_killed == Some(true) {
        field("oom killed", &cb("yes — hit the memory limit", p().red));
    }
    if let Some(err) = st.error.as_ref().filter(|e| !e.is_empty()) {
        field("error", &c(err, p().red));
    }
    let restarts = ct.restart_count.unwrap_or(0);
    if restarts > 0 {
        field(
            "restarts",
            &c(&restarts.to_string(), if restarts > 3 { p().yellow } else { p().fg }),
        );
    }

    if let Some(h) = &st.health {
        let hs = h.status.map(|s| s.to_string()).unwrap_or_default();
        let (hcol, hglyph) = theme::health_style(&hs).unwrap_or((p().gray, "·"));
        let streak = h.failing_streak.unwrap_or(0);
        let extra = if streak > 0 { format!(" · {streak} failing") } else { String::new() };
        field("health", &format!("{} {}{}", c(hglyph, hcol), c(&hs, hcol), dim(&extra)));
        // The last probe's output is the single most useful line when a
        // healthcheck is failing, and `docker inspect` buries it.
        if let Some(last) = h.log.as_ref().and_then(|l| l.last()) {
            let out = last.output.clone().unwrap_or_default();
            let out = out.lines().next().unwrap_or("").trim().to_string();
            if !out.is_empty() {
                field("last probe", &dim(&fmt::truncate(&out, 100)));
            }
        }
    }
}

fn config(ct: &ContainerInspectResponse, show_secrets: bool, show_env: bool) {
    let Some(cfg) = &ct.config else { return };
    section("config", p().magenta);
    if let Some(ep) = &cfg.entrypoint {
        field("entrypoint", &c(&ep.join(" "), p().fg));
    }
    if let Some(cmd) = &cfg.cmd {
        field("command", &c(&cmd.join(" "), p().fg));
    }
    if let Some(wd) = cfg.working_dir.as_ref().filter(|w| !w.is_empty()) {
        field("workdir", &dim(wd));
    }
    if let Some(u) = cfg.user.as_ref().filter(|u| !u.is_empty()) {
        field(
            "user",
            &c(u, if u.as_str() == "root" || u.as_str() == "0" { p().yellow } else { p().fg }),
        );
    }
    if let Some(hc) = &cfg.healthcheck
        && let Some(test) = &hc.test
    {
        let probe = test.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        let every = hc.interval.map(|n| fmt::age(n / 1_000_000_000)).unwrap_or_default();
        field(
            "healthcheck",
            &format!("{} {}", dim(&fmt::truncate(&probe, 70)), dim(&format!("every {every}"))),
        );
    }
    if let Some(sig) = cfg.stop_signal.as_ref().filter(|s| !s.is_empty()) {
        field("stop signal", &dim(sig));
    }

    let Some(env) = &cfg.env else { return };
    if env.is_empty() {
        return;
    }
    if !show_env {
        field("env", &dim(&format!("{} variables (--env to show)", env.len())));
        return;
    }
    let mut rows = Vec::new();
    for e in env {
        let (k, v) = e.split_once('=').unwrap_or((e.as_str(), ""));
        let upper = k.to_ascii_uppercase();
        let hidden = !show_secrets && SECRETISH.iter().any(|s| upper.contains(s));
        let shown = if hidden {
            c(&format!("•••• ({} chars)", v.len()), p().yellow)
        } else {
            c(&fmt::truncate(v, 90), p().fg)
        };
        rows.push(format!("{}{}{}", c(k, p().cyan), dim("="), shown));
    }
    list_field("env", &rows);
}

fn resources(ct: &ContainerInspectResponse) {
    let Some(hc) = &ct.host_config else { return };
    section("resources", p().orange);

    let mem = hc.memory.unwrap_or(0);
    field(
        "memory",
        &if mem > 0 {
            c(&fmt::bytes(mem as u64), theme::size_color(mem as u64))
        } else {
            dim("unlimited")
        },
    );
    if let Some(nano) = hc.nano_cpus.filter(|n| *n > 0) {
        field("cpus", &c(&format!("{:.2}", nano as f64 / 1e9), p().fg));
    }
    if let Some(cpus) = hc.cpuset_cpus.as_ref().filter(|s| !s.is_empty()) {
        field("cpuset", &dim(cpus));
    }
    if let Some(p) = hc.pids_limit.filter(|p| *p > 0) {
        field("pids limit", &dim(&p.to_string()));
    }
    if let Some(rp) = &hc.restart_policy {
        let name = rp.name.map(|n| n.to_string()).unwrap_or_default();
        let max = rp.maximum_retry_count.unwrap_or(0);
        let text = if max > 0 { format!("{name} (max {max})") } else { name };
        field("restart", &c(&text, if text.is_empty() { p().gray } else { p().fg }));
    }
    if let Some(lc) = &hc.log_config
        && let Some(driver) = &lc.typ
    {
        field("log driver", &dim(&driver.to_string()));
    }

    // Anything that widens the blast radius gets called out in colour.
    if hc.privileged == Some(true) {
        field("privileged", &cb("yes — full host access", p().red));
    }
    if let Some(caps) = hc.cap_add.as_ref().filter(|v| !v.is_empty()) {
        field("cap add", &c(&caps.join(" "), p().yellow));
    }
    if let Some(caps) = hc.cap_drop.as_ref().filter(|v| !v.is_empty()) {
        field("cap drop", &dim(&caps.join(" ")));
    }
    if hc.readonly_rootfs == Some(true) {
        field("rootfs", &c("read-only", p().green));
    }
    if hc.auto_remove == Some(true) {
        field("auto remove", &yes_no(true, p().yellow, p().gray));
    }
}

fn network(ct: &ContainerInspectResponse) {
    let Some(ns) = &ct.network_settings else { return };
    section("network", p().cyan);
    if let Some(hc) = &ct.host_config
        && let Some(mode) = &hc.network_mode
    {
        field("mode", &dim(mode));
    }

    if let Some(ports) = &ns.ports {
        let mut rows = Vec::new();
        for (private, bindings) in ports {
            match bindings {
                Some(bs) if !bs.is_empty() => {
                    for b in bs {
                        let host_ip = b.host_ip.clone().unwrap_or_default();
                        let host_port = b.host_port.clone().unwrap_or_default();
                        let host = if host_ip.is_empty() || host_ip == "0.0.0.0" {
                            format!(":{host_port}")
                        } else if host_ip.contains(':') {
                            format!("[{host_ip}]:{host_port}") // IPv6 needs brackets
                        } else {
                            format!("{host_ip}:{host_port}")
                        };
                        rows.push(format!(
                            "{} {} {}",
                            c(&host, p().cyan),
                            dim(g().arrow),
                            c(private, p().fg)
                        ));
                    }
                }
                _ => rows.push(format!("{} {}", dim("exposed"), c(private, p().gray))),
            }
        }
        rows.sort();
        list_field("ports", &rows);
    }

    if let Some(nets) = &ns.networks {
        let mut rows = Vec::new();
        for (name, ep) in nets {
            let ip = ep.ip_address.clone().unwrap_or_default();
            let aliases = ep
                .aliases
                .clone()
                .unwrap_or_default()
                .into_iter()
                .filter(|a| !a.is_empty())
                .collect::<Vec<_>>();
            let alias_txt = if aliases.is_empty() {
                String::new()
            } else {
                format!(" · {}", aliases.join(" "))
            };
            rows.push(format!("{} {}{}", c(name, p().cyan), dim(&ip), dim(&alias_txt)));
        }
        rows.sort();
        list_field("networks", &rows);
    }
}

fn mounts(ct: &ContainerInspectResponse) {
    let Some(ms) = ct.mounts.as_ref().filter(|m| !m.is_empty()) else { return };
    section("mounts", p().green);
    let mut rows = Vec::new();
    for m in ms {
        let kind = m.typ.clone().unwrap_or_default();
        let src = m.name.clone().or_else(|| m.source.clone()).unwrap_or_default();
        let dst = m.destination.clone().unwrap_or_default();
        let rw = m.rw.unwrap_or(true);
        rows.push(format!(
            "{} {} {} {} {}",
            c(&fmt::pad(&kind, 6), if kind == "volume" { p().magenta } else { p().blue }),
            c(&fmt::truncate(&src, 44), p().fg),
            dim(g().arrow),
            c(&dst, p().cyan),
            if rw { dim("rw") } else { c("ro", p().yellow) }
        ));
    }
    rows.sort();
    list_field("", &rows);
}

fn labels(ct: &ContainerInspectResponse) {
    let Some(labels) = ct.config.as_ref().and_then(|c| c.labels.as_ref()) else { return };
    if labels.is_empty() {
        return;
    }
    section("labels", p().gray);
    // Compose metadata first — it is what identifies where this came from.
    let mut compose: Vec<String> = Vec::new();
    let mut other: Vec<String> = Vec::new();
    for (k, v) in labels {
        let row = format!("{}{}{}", c(k, p().cyan), dim("="), c(&fmt::truncate(v, 80), p().fg));
        if k.starts_with("com.docker.compose.") {
            compose.push(row);
        } else {
            other.push(row);
        }
    }
    compose.sort();
    other.sort();
    list_field("", &compose);
    list_field("", &other);
}
