//! `dok prune` — what `docker system prune` would remove, before you run it.
//!
//! dok reads, docker writes. This never deletes anything; it shows the exact
//! set `docker system prune` would take, grouped by kind and sized, and then
//! prints the docker command that does the removing. The one thing a preview
//! must get right is the boundary: `docker system prune` takes *dangling*
//! images, not every unused one, unless it is told `-a`. The same flag here
//! draws the same line.

use anyhow::Result;
use bollard::models::ContainerSummary;

use crate::cmds::df::{self, Item};
use crate::dk;
use crate::fmt;
use crate::json;
use crate::theme::{self, *};

struct Group {
    label: &'static str,
    /// What the row count is counting: "stopped", "dangling", "unused"…
    kind: &'static str,
    color: Rgb,
    items: Vec<Item>,
}

pub async fn run(all: bool, volumes: bool) -> Result<()> {
    let docker = dk::connect()?;
    let usage = dk::df(&docker).await?;
    let containers = dk::containers(&docker, true).await?;

    let mut groups = Vec::new();

    // Containers: every one that is not running.
    if let Some(u) = &usage.container_usage {
        let items = u.items.as_deref().map(df::container_items).unwrap_or_default();
        groups.push(Group {
            label: "containers",
            kind: "stopped",
            color: p().blue,
            items: items.into_iter().filter(|i| i.reclaimable).collect(),
        });
    }

    // Images: dangling by default, every unused one with -a. This is the
    // line docker draws, and the reason to preview at all.
    if let Some(u) = &usage.image_usage {
        let items = u.items.as_deref().map(df::image_items).unwrap_or_default();
        groups.push(Group {
            label: "images",
            kind: if all { "unused" } else { "dangling" },
            color: p().magenta,
            items: items.into_iter().filter(|i| image_selected(i, all)).collect(),
        });
    }

    // Networks: no container attached, running or not, and not one of the
    // three built-ins docker refuses to remove.
    groups.push(Group {
        label: "networks",
        kind: "unused",
        color: p().cyan,
        items: unused_networks(&docker, &containers).await?,
    });

    if let Some(u) = &usage.build_cache_usage {
        let items = u.items.as_deref().map(df::cache_items).unwrap_or_default();
        groups.push(Group {
            label: "build cache",
            kind: "unused",
            color: p().yellow,
            items: items.into_iter().filter(|i| i.reclaimable).collect(),
        });
    }

    // Volumes only on request, exactly as docker does: data is not cache.
    if volumes && let Some(u) = &usage.volume_usage {
        let items = u.items.as_deref().map(df::volume_items).unwrap_or_default();
        groups.push(Group {
            label: "volumes",
            kind: "unused",
            color: p().green,
            items: items.into_iter().filter(|i| i.reclaimable).collect(),
        });
    }

    let total: i64 = groups.iter().flat_map(|g| g.items.iter()).map(|i| i.size.max(0)).sum();
    let count: usize = groups.iter().map(|g| g.items.len()).sum();

    if json::enabled() {
        json::emit(&serde_json::json!({
            "groups": groups.iter().map(|g| serde_json::json!({
                "kind": g.label,
                "reason": g.kind,
                "count": g.items.len(),
                "size": json::size(g.items.iter().map(|i| i.size.max(0)).sum::<i64>()),
                "items": g.items.iter().map(|i| serde_json::json!({
                    "name": i.name,
                    "size": json::size(i.size),
                    "note": json::opt(&i.note),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "total": json::size(total),
            "count": count,
            // Stated rather than implied: this command never removes anything.
            "removed": false,
            "command": format!(
                "docker system prune{}{}",
                if all { " -a" } else { "" },
                if volumes { " --volumes" } else { "" }
            ),
        }));
        return Ok(());
    }

    for g in &groups {
        print_group(g);
    }

    println!();
    if count == 0 {
        println!("{}", dim("nothing to reclaim"));
    } else {
        println!(
            "{} {} {}",
            c(&fmt::bytes(total.max(0) as u64), theme::size_color(total.max(0) as u64)),
            c("reclaimable", p().fg),
            dim(&format!(
                "· {count} item{} · nothing was removed",
                if count == 1 { "" } else { "s" }
            ))
        );
    }

    // The command that actually does it, shaped to match what was previewed.
    let mut flags = String::new();
    if all {
        flags.push_str(" -a");
    }
    if volumes {
        flags.push_str(" --volumes");
    }
    println!();
    println!("  {} {}", dim("$"), c(&format!("docker system prune{flags}"), p().fg));
    Ok(())
}

fn print_group(grp: &Group) {
    let size: i64 = grp.items.iter().map(|i| i.size.max(0)).sum();
    let n = grp.items.len();
    let head = format!(
        "{} {}  {}{}",
        c(theme::icon(g().bullet, "\u{f0a0}"), grp.color),
        cb(grp.label, grp.color),
        dim(&format!("{n} {}", grp.kind)),
        if size > 0 {
            format!(" {} {}", dim("·"), c(&fmt::bytes(size as u64), theme::size_color(size as u64)))
        } else {
            String::new()
        }
    );
    println!("{}", head.trim_end());

    if n == 0 {
        return;
    }
    // Biggest first, since that is what someone is deciding about.
    let mut items: Vec<&Item> = grp.items.iter().collect();
    items.sort_by_key(|i| -i.size);
    let last = dim(g().tree_last);
    let branch = dim(g().tree_branch);
    for (i, it) in items.iter().enumerate() {
        let stub = if i + 1 == n { &last } else { &branch };
        let size = if it.size > 0 {
            c(&fmt::bytes(it.size as u64), theme::size_color(it.size as u64))
        } else {
            dim("—")
        };
        println!(
            "{} {}  {}  {}",
            stub,
            fmt::pad(&c(&fmt::truncate(&it.name, 40), p().fg), 40),
            fmt::rpad(&size, 7),
            dim(&it.note)
        );
    }
}

/// The line `docker system prune` draws: dangling images always, merely
/// unused ones only with `-a`. An image with a container is never taken.
fn image_selected(item: &Item, all: bool) -> bool {
    item.reclaimable && (all || item.dangling)
}

/// The three networks docker refuses to remove.
fn is_builtin_network(name: &str) -> bool {
    matches!(name, "bridge" | "host" | "none")
}

/// Networks nothing is attached to. The list endpoint does not report
/// membership, so it is reconstructed from every container's own settings —
/// the same way `dok tree` does it.
async fn unused_networks(
    docker: &bollard::Docker,
    containers: &[ContainerSummary],
) -> Result<Vec<Item>> {
    let nets = dk::networks(docker).await?;
    let mut out = Vec::new();
    for net in nets {
        let name = net.name.clone().unwrap_or_default();
        // docker will not remove these, so previewing them would be a lie.
        if is_builtin_network(&name) {
            continue;
        }
        let attached = containers
            .iter()
            .filter(|ct| {
                ct.network_settings
                    .as_ref()
                    .and_then(|s| s.networks.as_ref())
                    .is_some_and(|m| m.contains_key(&name))
            })
            .count();
        if attached == 0 {
            out.push(Item {
                name,
                size: 0,
                reclaimable: true,
                note: net.driver.unwrap_or_default(),
                dangling: false,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(reclaimable: bool, dangling: bool) -> Item {
        Item { name: String::new(), size: 0, reclaimable, dangling, note: String::new() }
    }

    /// The whole reason to preview: without -a, a tagged image that nothing
    /// runs is left alone, exactly as docker leaves it.
    #[test]
    fn unused_but_tagged_images_need_dash_a() {
        assert!(!image_selected(&img(true, false), false));
        assert!(image_selected(&img(true, false), true));
    }

    #[test]
    fn dangling_images_go_either_way() {
        assert!(image_selected(&img(true, true), false));
        assert!(image_selected(&img(true, true), true));
    }

    #[test]
    fn an_image_in_use_is_never_taken() {
        assert!(!image_selected(&img(false, true), true));
        assert!(!image_selected(&img(false, false), true));
    }

    #[test]
    fn builtin_networks_are_never_previewed() {
        for n in ["bridge", "host", "none"] {
            assert!(is_builtin_network(n));
        }
        assert!(!is_builtin_network("old-release_default"));
    }
}
