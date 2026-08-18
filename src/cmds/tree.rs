//! `dok tree` — compose projects, networks and volumes as box-drawing trees.

use anyhow::Result;
use bollard::models::ContainerSummary;
use clap::ValueEnum;
use std::collections::BTreeMap;

use crate::dk;
use crate::fmt;
use crate::theme::{self, *};

#[derive(Copy, Clone, ValueEnum, PartialEq, Eq)]
pub enum Section {
    Projects,
    Networks,
    Volumes,
}

pub async fn run(only: Option<Section>, all: bool) -> Result<()> {
    let docker = dk::connect()?;
    let containers = dk::containers(&docker, all).await?;

    let want = |s: Section| only.is_none() || only == Some(s);
    let mut printed = false;

    if want(Section::Projects) {
        projects(&containers);
        printed = true;
    }
    if want(Section::Networks) {
        if printed {
            println!();
        }
        networks(&docker, &containers).await?;
        printed = true;
    }
    if want(Section::Volumes) {
        if printed {
            println!();
        }
        volumes(&docker, &containers).await?;
    }
    Ok(())
}

// ── shapes ──────────────────────────────────────────────────────────────────

fn tee(last: bool) -> &'static str {
    if last { g().tree_last } else { g().tree_branch }
}

fn section(title: &str, icon: &str, col: Rgb) -> String {
    format!("{} {}", c(icon, col), cb(title, col))
}

/// Column widths for a set of leaves, so names and images line up.
struct Cols {
    name: usize,
    image: usize,
}

fn cols<'a, I: IntoIterator<Item = &'a ContainerSummary>>(it: I) -> Cols {
    let mut c = Cols { name: 0, image: 0 };
    for ct in it {
        c.name = c.name.max(fmt::visible_width(&name_of(ct)));
        c.image = c.image.max(fmt::visible_width(&image_of(ct)));
    }
    c
}

/// Display name: swarm task ids dropped, long names cut, so one outlier
/// cannot push the image column off the screen.
fn name_of(ct: &ContainerSummary) -> String {
    fmt::truncate(&fmt::short_task_name(&dk::name_of(ct)), 36)
}

fn image_of(ct: &ContainerSummary) -> String {
    fmt::truncate(ct.image.as_deref().unwrap_or(""), 40)
}

/// A leaf: state dot, name, image — both padded to the section's widths.
fn node(ct: &ContainerSummary, w: &Cols) -> String {
    let state = dk::state_of(ct);
    let (scol, glyph) = theme::state_style(&state);
    let name = name_of(ct);
    format!(
        "{} {} {}",
        c(glyph, scol),
        fmt::pad(&c(&name, if state == "running" { p().fg } else { p().gray }), w.name),
        fmt::pad(&dim(&image_of(ct)), w.image)
    )
}

// ── sections ────────────────────────────────────────────────────────────────

fn projects(containers: &[ContainerSummary]) {
    println!("{}", section("projects", theme::icon(g().group_mark, "\u{f0e8}"), p().blue));

    let mut groups: BTreeMap<String, Vec<&ContainerSummary>> = BTreeMap::new();
    for ct in containers {
        let key = dk::label(ct, dk::COMPOSE_PROJECT).unwrap_or("standalone").to_string();
        groups.entry(key).or_default().push(ct);
    }
    if groups.is_empty() {
        println!("{}", dim("  (none)"));
        return;
    }

    let keys: Vec<&String> = groups.keys().collect();
    for (gi, key) in keys.iter().enumerate() {
        let last_group = gi + 1 == keys.len();
        let members = &groups[*key];
        let running = members.iter().filter(|c| dk::state_of(c) == "running").count();
        println!(
            "{}{} {}",
            dim(tee(last_group)),
            cb(key, if **key == "standalone" { p().gray } else { p().blue }),
            dim(&format!("· {running}/{} up", members.len()))
        );
        let stem = if last_group { g().tree_blank } else { g().tree_stem };
        let w = cols(members.iter().copied());
        for (i, ct) in members.iter().enumerate() {
            println!("{}{}{}", dim(stem), dim(tee(i + 1 == members.len())), node(ct, &w));
        }
    }
}

async fn networks(docker: &bollard::Docker, containers: &[ContainerSummary]) -> Result<()> {
    println!("{}", section("networks", theme::icon(g().bullet, "\u{f0e8}"), p().cyan));
    let mut nets = dk::networks(docker).await?;
    nets.sort_by_key(|n| n.name.clone().unwrap_or_default());

    for (ni, net) in nets.iter().enumerate() {
        let last_net = ni + 1 == nets.len();
        let name = net.name.clone().unwrap_or_default();
        let driver = net.driver.clone().unwrap_or_default();
        let subnet = net
            .ipam
            .as_ref()
            .and_then(|i| i.config.as_ref())
            .and_then(|cfgs| cfgs.first())
            .and_then(|cfg| cfg.subnet.clone())
            .unwrap_or_default();

        // Containers attached to this network, with their address on it.
        let members: Vec<(&ContainerSummary, String)> = containers
            .iter()
            .filter_map(|ct| {
                let ep = ct.network_settings.as_ref()?.networks.as_ref()?.get(&name)?;
                let ip = ep.ip_address.clone().unwrap_or_default();
                Some((ct, ip))
            })
            .collect();

        let mut meta = vec![driver];
        if !subnet.is_empty() {
            meta.push(subnet);
        }
        if net.internal.unwrap_or(false) {
            meta.push("internal".into());
        }
        println!(
            "{}{} {}",
            dim(tee(last_net)),
            cb(&name, p().cyan),
            dim(&format!("· {}", meta.join(" · ")))
        );

        let stem = if last_net { g().tree_blank } else { g().tree_stem };
        if members.is_empty() {
            println!("{}{}{}", dim(stem), dim(tee(true)), dim("(no containers)"));
            continue;
        }
        let w = cols(members.iter().map(|(ct, _)| *ct));
        for (i, (ct, ip)) in members.iter().enumerate() {
            println!(
                "{}{}{} {}",
                dim(stem),
                dim(tee(i + 1 == members.len())),
                node(ct, &w),
                dim(ip)
            );
        }
    }
    Ok(())
}

async fn volumes(docker: &bollard::Docker, containers: &[ContainerSummary]) -> Result<()> {
    println!("{}", section("volumes", theme::icon(g().bullet, "\u{f0a0}"), p().magenta));
    let mut vols = dk::volumes(docker).await?;
    vols.sort_by(|a, b| a.name.cmp(&b.name));

    if vols.is_empty() {
        println!("{}", dim("  (none)"));
        return Ok(());
    }

    for (vi, v) in vols.iter().enumerate() {
        let last = vi + 1 == vols.len();
        let size = v.usage_data.as_ref().map(|u| u.size).filter(|s| *s >= 0);

        // Who mounts it, and where.
        let users: Vec<(&ContainerSummary, String, bool)> = containers
            .iter()
            .filter_map(|ct| {
                let m = ct.mounts.as_ref()?.iter().find(|m| m.name.as_deref() == Some(&v.name))?;
                Some((ct, m.destination.clone().unwrap_or_default(), m.rw.unwrap_or(true)))
            })
            .collect();

        let mut meta = vec![v.driver.clone()];
        if let Some(sz) = size {
            meta.push(fmt::bytes(sz as u64));
        }
        if users.is_empty() {
            meta.push("unused".into());
        }
        let name_col = if users.is_empty() { p().gray } else { p().magenta };
        println!(
            "{}{} {}",
            dim(tee(last)),
            cb(&fmt::truncate(&v.name, 48), name_col),
            dim(&format!("· {}", meta.join(" · ")))
        );

        let stem = if last { g().tree_blank } else { g().tree_stem };
        let w = cols(users.iter().map(|(ct, _, _)| *ct));
        for (i, (ct, dest, rw)) in users.iter().enumerate() {
            println!(
                "{}{}{} {} {}",
                dim(stem),
                dim(tee(i + 1 == users.len())),
                node(ct, &w),
                c(dest, p().cyan),
                if *rw { dim("rw") } else { c("ro", p().yellow) }
            );
        }
    }
    Ok(())
}
