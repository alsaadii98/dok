//! `dok top` — processes inside containers, as a tree.

use anyhow::Result;
use bollard::query_parameters::TopOptionsBuilder;
use std::collections::HashMap;

use crate::dk;
use crate::fmt;
use crate::json;
use crate::table::{Column, Table};
use crate::theme::{self, *};

pub async fn run(wanted: Vec<String>, ps_args: Option<String>, flat: bool) -> Result<()> {
    let docker = dk::connect()?;

    let targets: Vec<String> = if wanted.is_empty() {
        dk::containers(&docker, false).await?.iter().map(dk::name_of).collect()
    } else {
        let mut out = Vec::new();
        for w in &wanted {
            out.push(dk::resolve(&docker, w).await?);
        }
        out
    };

    if targets.is_empty() {
        if json::enabled() {
            json::emit(&serde_json::json!({ "containers": [] }));
        } else {
            println!("{}", dim("no running containers"));
        }
        return Ok(());
    }

    // `-o` picks the columns we can actually draw a tree from.
    let args = ps_args.unwrap_or_else(|| "-eo pid,ppid,user,pcpu,pmem,etime,args".into());
    let opts = TopOptionsBuilder::default().ps_args(&args).build();

    let mut doc = Vec::new();

    for (i, name) in targets.iter().enumerate() {
        if i > 0 && !json::enabled() {
            println!();
        }
        let top = if crate::demo::enabled() {
            Ok(crate::demo::top(name))
        } else {
            docker.top_processes(name, Some(opts.clone())).await
        };
        let top = match top {
            Ok(t) => t,
            Err(e) => {
                if json::enabled() {
                    doc.push(serde_json::json!({ "container": name, "error": e.to_string() }));
                } else {
                    println!(
                        "{} {}",
                        cb(name, theme::hash_color(name)),
                        c(&format!("· {e}"), p().red)
                    );
                }
                continue;
            }
        };
        let titles = top.titles.unwrap_or_default();
        let procs = top.processes.unwrap_or_default();
        if json::enabled() {
            // Column names come from whatever `ps_args` asked for, so the keys
            // are docker's titles lowercased rather than a fixed set.
            doc.push(serde_json::json!({
                "container": name,
                "titles": titles,
                "processes": procs.iter().map(|row| {
                    titles
                        .iter()
                        .zip(row.iter())
                        .map(|(t, v)| (t.to_lowercase(), serde_json::Value::from(v.as_str())))
                        .collect::<serde_json::Map<_, _>>()
                }).collect::<Vec<_>>(),
            }));
            continue;
        }
        println!(
            "{} {}",
            cb(name, theme::hash_color(name)),
            dim(&format!("· {} process{}", procs.len(), if procs.len() == 1 { "" } else { "es" }))
        );
        render(&titles, &procs, flat);
    }
    if json::enabled() {
        json::emit(&serde_json::json!({ "containers": doc }));
    }
    Ok(())
}

fn col_index(titles: &[String], want: &[&str]) -> Option<usize> {
    titles.iter().position(|t| want.contains(&t.to_ascii_uppercase().as_str()))
}

fn render(titles: &[String], procs: &[Vec<String>], flat: bool) {
    if procs.is_empty() {
        println!("{}", dim("  (none)"));
        return;
    }

    let pid_i = col_index(titles, &["PID"]);
    let ppid_i = col_index(titles, &["PPID"]);
    let cpu_i = col_index(titles, &["%CPU", "PCPU", "C"]);
    let mem_i = col_index(titles, &["%MEM", "PMEM"]);
    let cmd_i = col_index(titles, &["COMMAND", "CMD", "ARGS"]);

    // Column set: everything except the command, which is rendered last and
    // carries the tree prefix.
    let mut cols: Vec<Column> = Vec::new();
    let mut order: Vec<usize> = Vec::new();
    for (i, t) in titles.iter().enumerate() {
        if Some(i) == cmd_i {
            continue;
        }
        let numeric = Some(i) == cpu_i || Some(i) == mem_i || Some(i) == pid_i || Some(i) == ppid_i;
        cols.push(if numeric { Column::right(t.clone()) } else { Column::left(t.clone()) });
        order.push(i);
    }
    cols.push(Column::left("COMMAND").flex(20));

    let rows = match (flat, pid_i, ppid_i) {
        (false, Some(pid), Some(ppid)) => tree_order(procs, pid, ppid),
        _ => procs.iter().map(|p| (0usize, p)).collect::<Vec<_>>(),
    };

    let mut t = Table::new(cols);
    for (depth, proc) in rows {
        let mut cells: Vec<String> = Vec::new();
        for &i in &order {
            let raw = proc.get(i).cloned().unwrap_or_default();
            let cell = if Some(i) == cpu_i || Some(i) == mem_i {
                c(&raw, load_color(&raw))
            } else if Some(i) == pid_i {
                c(&raw, p().fg)
            } else {
                dim(&raw)
            };
            cells.push(cell);
        }
        let cmd = cmd_i.and_then(|i| proc.get(i)).cloned().unwrap_or_default();
        cells.push(format!("{}{}", dim(&"  ".repeat(depth)), command_cell(&cmd)));
        t.row(cells);
    }
    t.print();
}

/// Depth-first ordering by PPID so children sit under their parent.
fn tree_order(procs: &[Vec<String>], pid_i: usize, ppid_i: usize) -> Vec<(usize, &Vec<String>)> {
    let pids: Vec<&str> =
        procs.iter().map(|p| p.get(pid_i).map(String::as_str).unwrap_or("")).collect();
    let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for (idx, p) in procs.iter().enumerate() {
        let ppid = p.get(ppid_i).map(String::as_str).unwrap_or("");
        if pids.contains(&ppid) {
            children.entry(ppid).or_default().push(idx);
        } else {
            roots.push(idx);
        }
    }

    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
    let mut seen = vec![false; procs.len()];
    while let Some((idx, depth)) = stack.pop() {
        if seen[idx] {
            continue; // guards against a PID cycle in exotic ps output
        }
        seen[idx] = true;
        out.push((depth, &procs[idx]));
        if let Some(kids) = children.get(pids[idx]) {
            for &k in kids.iter().rev() {
                stack.push((k, depth + 1));
            }
        }
    }
    // Anything unreachable (cycle) still gets printed.
    for (idx, p) in procs.iter().enumerate() {
        if !seen[idx] {
            out.push((0, p));
        }
    }
    out
}

fn load_color(raw: &str) -> Rgb {
    match raw.parse::<f64>().unwrap_or(0.0) {
        v if v >= 50.0 => p().red,
        v if v >= 20.0 => p().yellow,
        v if v >= 1.0 => p().fg,
        _ => p().gray,
    }
}

/// Bright binary, dim arguments — the shape of the command at a glance.
fn command_cell(cmd: &str) -> String {
    let mut parts = cmd.splitn(2, ' ');
    let head = parts.next().unwrap_or("");
    let bin = head.rsplit('/').next().unwrap_or(head);
    let prefix = &head[..head.len() - bin.len()];
    let rest = parts.next().unwrap_or("");
    let mut out = format!("{}{}", dim(prefix), c(bin, p().fg));
    if !rest.is_empty() {
        out.push(' ');
        out.push_str(&dim(&fmt::truncate(rest, 80)));
    }
    out
}
