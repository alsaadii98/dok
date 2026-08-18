//! `dok df` — where the disk went, and how much you can get back.

use anyhow::Result;
use serde_json::Value;

use crate::dk;
use crate::fmt;
use crate::table::{Column, Table};
use crate::theme::{self, *};

struct Section {
    label: &'static str,
    icon: &'static str,
    color: Rgb,
    total: i64,
    reclaimable: i64,
    active: i64,
    count: i64,
    items: Vec<Item>,
}

/// A single image / container / volume / cache entry, flattened from the
/// untyped `items` the API returns.
struct Item {
    name: String,
    size: i64,
    reclaimable: bool,
    note: String,
}

pub async fn run(verbose: bool, top: usize) -> Result<()> {
    let docker = dk::connect()?;
    let usage = dk::df(&docker).await?;

    let mut sections = Vec::new();
    if let Some(u) = &usage.image_usage {
        sections.push(Section {
            label: "images",
            icon: theme::icon(g().bullet, "\u{f0a0}"),
            color: p().magenta,
            total: u.total_size.unwrap_or(0),
            reclaimable: u.reclaimable.unwrap_or(0),
            active: u.active_count.unwrap_or(0),
            count: u.total_count.unwrap_or(0),
            items: u.items.as_deref().map(image_items).unwrap_or_default(),
        });
    }
    if let Some(u) = &usage.container_usage {
        sections.push(Section {
            label: "containers",
            icon: theme::icon(g().dot_stopped, "\u{f21b}"),
            color: p().blue,
            total: u.total_size.unwrap_or(0),
            reclaimable: u.reclaimable.unwrap_or(0),
            active: u.active_count.unwrap_or(0),
            count: u.total_count.unwrap_or(0),
            items: u.items.as_deref().map(container_items).unwrap_or_default(),
        });
    }
    if let Some(u) = &usage.volume_usage {
        sections.push(Section {
            label: "volumes",
            icon: theme::icon(g().bullet, "\u{f0a0}"),
            color: p().cyan,
            total: u.total_size.unwrap_or(0),
            reclaimable: u.reclaimable.unwrap_or(0),
            active: u.active_count.unwrap_or(0),
            count: u.total_count.unwrap_or(0),
            items: u.items.as_deref().map(volume_items).unwrap_or_default(),
        });
    }
    if let Some(u) = &usage.build_cache_usage {
        sections.push(Section {
            label: "build cache",
            icon: theme::icon(g().dot_removing, "\u{f085}"),
            color: p().orange,
            total: u.total_size.unwrap_or(0),
            reclaimable: u.reclaimable.unwrap_or(0),
            active: u.active_count.unwrap_or(0),
            count: u.total_count.unwrap_or(0),
            items: u.items.as_deref().map(cache_items).unwrap_or_default(),
        });
    }

    let grand: i64 = sections.iter().map(|s| s.total).sum();
    let grand_reclaim: i64 = sections.iter().map(|s| s.reclaimable).sum();

    let mut t = Table::new(vec![
        Column::left(""),
        Column::left("TYPE"),
        Column::right("COUNT"),
        Column::right("ACTIVE"),
        Column::right("SIZE"),
        Column::right("RECLAIMABLE"),
        Column::left(""),
    ]);

    for s in &sections {
        let pct = if s.total > 0 { s.reclaimable as f64 / s.total as f64 * 100.0 } else { 0.0 };
        t.row(vec![
            c(s.icon, s.color),
            c(s.label, s.color),
            c(&s.count.to_string(), p().fg),
            dim(&s.active.to_string()),
            c(&fmt::bytes(s.total.max(0) as u64), theme::size_color(s.total.max(0) as u64)),
            if s.reclaimable > 0 {
                c(&format!("{} ({pct:.0}%)", fmt::bytes(s.reclaimable as u64)), p().orange)
            } else {
                dim("—")
            },
            // Bar shows this section's share of total disk, split used|reclaimable.
            share_bar(s.total, s.reclaimable, grand, s.color),
        ]);
    }
    t.print();

    let pct = if grand > 0 { grand_reclaim as f64 / grand as f64 * 100.0 } else { 0.0 };
    println!(
        "\n{} {}  {} {}",
        dim("total"),
        cb(&fmt::bytes(grand.max(0) as u64), p().fg),
        dim("· reclaimable"),
        cb(&format!("{} ({pct:.0}%)", fmt::bytes(grand_reclaim.max(0) as u64)), p().orange)
    );
    if grand_reclaim > 0 {
        println!("{}", dim("run `docker system prune -a --volumes` to reclaim"));
    }

    if verbose {
        for s in &sections {
            if s.items.is_empty() {
                continue;
            }
            println!("\n{} {}", c(s.icon, s.color), cb(s.label, s.color));
            let mut items: Vec<&Item> = s.items.iter().collect();
            items.sort_by_key(|i| -i.size);
            let shown = items.len().min(top);
            let mut it = Table::new(vec![
                Column::left("NAME").flex(20).cap(40),
                Column::right("SIZE"),
                Column::left("STATUS"),
            ]);
            for i in items.iter().take(shown) {
                it.row(vec![
                    c(&i.name, if i.reclaimable { p().gray } else { p().fg }),
                    c(&fmt::bytes(i.size.max(0) as u64), theme::size_color(i.size.max(0) as u64)),
                    if i.reclaimable {
                        c("reclaimable", p().orange)
                    } else {
                        c(&i.note, p().green)
                    },
                ]);
            }
            it.print();
            if items.len() > shown {
                println!("{}", dim(&format!("… {} more", items.len() - shown)));
            }
        }
    }

    Ok(())
}

/// `████░░░░········` — filled = in use, hatched = reclaimable, dotted = rest.
fn share_bar(total: i64, reclaimable: i64, grand: i64, col: Rgb) -> String {
    const WIDTH: usize = 24;
    if grand <= 0 || total <= 0 {
        return dim(&g().bar_empty.repeat(WIDTH));
    }
    let cells = ((total as f64 / grand as f64) * WIDTH as f64).round() as usize;
    let cells = cells.clamp(1, WIDTH);
    let reclaim_cells = ((reclaimable as f64 / total as f64) * cells as f64).round() as usize;
    let used_cells = cells - reclaim_cells.min(cells);
    format!(
        "{}{}{}",
        c(&g().bar_full.repeat(used_cells), col),
        c(&g().bar_alt.repeat(reclaim_cells.min(cells)), p().orange),
        dim(&g().bar_empty.repeat(WIDTH - cells))
    )
}

// ── item flattening ─────────────────────────────────────────────────────────

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}
fn n(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn image_items(items: &[Value]) -> Vec<Item> {
    items
        .iter()
        .map(|v| {
            let tags: Vec<&str> = v
                .get("RepoTags")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let containers = n(v, "Containers");
            Item {
                name: tags
                    .first()
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| format!("<none> {}", fmt::short_id(&s(v, "Id")))),
                // Shared layers make per-image size overlap; SharedSize is the
                // part another image also holds.
                size: n(v, "Size"),
                reclaimable: containers <= 0,
                note: format!("{containers} container{}", if containers == 1 { "" } else { "s" }),
            }
        })
        .collect()
}

fn container_items(items: &[Value]) -> Vec<Item> {
    items
        .iter()
        .map(|v| {
            let name = v
                .get("Names")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_start_matches('/')
                .to_string();
            let state = s(v, "State");
            Item {
                name: if name.is_empty() { fmt::short_id(&s(v, "Id")) } else { name },
                size: n(v, "SizeRw"),
                reclaimable: state != "running",
                note: state,
            }
        })
        .collect()
}

fn volume_items(items: &[Value]) -> Vec<Item> {
    items
        .iter()
        .map(|v| {
            let usage = v.get("UsageData");
            let refs = usage.map(|u| n(u, "RefCount")).unwrap_or(0);
            Item {
                name: s(v, "Name"),
                size: usage.map(|u| n(u, "Size")).unwrap_or(0),
                reclaimable: refs <= 0,
                note: format!("{refs} ref{}", if refs == 1 { "" } else { "s" }),
            }
        })
        .collect()
}

fn cache_items(items: &[Value]) -> Vec<Item> {
    items
        .iter()
        .map(|v| {
            let in_use = v.get("InUse").and_then(Value::as_bool).unwrap_or(false);
            let desc = s(v, "Description");
            Item {
                name: if desc.is_empty() { fmt::short_id(&s(v, "ID")) } else { desc },
                size: n(v, "Size"),
                reclaimable: !in_use,
                note: format!("used {}×", n(v, "UsageCount")),
            }
        })
        .collect()
}
