//! `dok stats` — a live CPU / memory / IO dashboard.

use anyhow::Result;
use bollard::Docker;
use bollard::query_parameters::StatsOptionsBuilder;
use futures_util::StreamExt;
use futures_util::future::join_all;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Padding, Paragraph, Row, Sparkline, Table};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::dk;
use crate::fmt;
use crate::json;
use crate::theme;

const HISTORY: usize = 120;

#[derive(Clone, Default)]
struct Sample {
    name: String,
    cpu: f64,
    mem: u64,
    mem_limit: u64,
    rx: u64,
    tx: u64,
    blk_r: u64,
    blk_w: u64,
    pids: u64,
}

#[derive(Default)]
struct Board {
    rows: Vec<Sample>,
    history: HashMap<String, Vec<u64>>,
    total_cpu: Vec<u64>,
    error: Option<String>,
    ticks: u64,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Sort {
    Cpu,
    Mem,
    Name,
}

impl Sort {
    fn next(self) -> Sort {
        match self {
            Sort::Cpu => Sort::Mem,
            Sort::Mem => Sort::Name,
            Sort::Name => Sort::Cpu,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Sort::Cpu => "cpu",
            Sort::Mem => "mem",
            Sort::Name => "name",
        }
    }
}

pub async fn run(wanted: Vec<String>, interval_ms: u64) -> Result<()> {
    if json::enabled() {
        anyhow::bail!(
            "dok stats is a live dashboard and has no JSON form — \
             use `dok ps --json` for a snapshot, or `docker stats --no-stream --format json`"
        );
    }
    let docker = dk::connect()?;
    // Fail before taking over the screen if the daemon is unreachable.
    docker.ping().await?;

    let board = Arc::new(Mutex::new(Board::default()));
    let sampler = tokio::spawn(sample_loop(
        docker.clone(),
        wanted,
        Duration::from_millis(interval_ms.max(300)),
        board.clone(),
    ));

    let mut term = ratatui::init();
    let mut sort = Sort::Cpu;
    let res = loop {
        {
            let b = board.lock().unwrap();
            if let Err(e) = term.draw(|f| draw(f, &b, sort)) {
                break Err(anyhow::Error::from(e));
            }
        }
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    break Ok(());
                }
                KeyCode::Char('s') => sort = sort.next(),
                _ => {}
            }
        }
    };

    ratatui::restore();
    sampler.abort();
    res
}

/// Poll every running container concurrently and fold the results into `board`.
async fn sample_loop(
    docker: Docker,
    wanted: Vec<String>,
    interval: Duration,
    board: Arc<Mutex<Board>>,
) {
    loop {
        let list = match dk::containers(&docker, false).await {
            Ok(l) => l,
            Err(e) => {
                board.lock().unwrap().error = Some(e.to_string());
                tokio::time::sleep(interval).await;
                continue;
            }
        };

        let targets: Vec<(String, String)> = list
            .iter()
            .filter_map(|ct| {
                let name = dk::name_of(ct);
                let id = ct.id.clone()?;
                let keep = wanted.is_empty()
                    || wanted
                        .iter()
                        .any(|w| name.contains(w.as_str()) || id.starts_with(w.as_str()));
                keep.then_some((name, id))
            })
            .collect();

        let futures = targets.into_iter().map(|(name, id)| {
            let docker = docker.clone();
            async move {
                let opts = StatsOptionsBuilder::default().stream(false).build();
                let mut s = docker.stats(&id, Some(opts)).take(1);
                match s.next().await {
                    Some(Ok(raw)) => Some(to_sample(name, &raw)),
                    _ => None,
                }
            }
        });

        let rows: Vec<Sample> = join_all(futures).await.into_iter().flatten().collect();

        {
            let mut b = board.lock().unwrap();
            b.error = None;
            b.ticks += 1;
            let mut total = 0.0;
            for r in &rows {
                let h = b.history.entry(r.name.clone()).or_default();
                h.push(r.cpu.round().max(0.0) as u64);
                if h.len() > HISTORY {
                    h.remove(0);
                }
                total += r.cpu;
            }
            b.total_cpu.push(total.round().max(0.0) as u64);
            if b.total_cpu.len() > HISTORY {
                b.total_cpu.remove(0);
            }
            let live: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
            b.history.retain(|k, _| live.contains(k));
            b.rows = rows;
        }

        tokio::time::sleep(interval).await;
    }
}

fn to_sample(name: String, s: &bollard::models::ContainerStatsResponse) -> Sample {
    let cpu = s.cpu_stats.as_ref();
    let pre = s.precpu_stats.as_ref();

    // Same formula the docker CLI uses.
    let cpu_delta = cpu
        .and_then(|c| c.cpu_usage.as_ref()?.total_usage)
        .unwrap_or(0)
        .saturating_sub(pre.and_then(|c| c.cpu_usage.as_ref()?.total_usage).unwrap_or(0))
        as f64;
    let sys_delta =
        cpu.and_then(|c| c.system_cpu_usage)
            .unwrap_or(0)
            .saturating_sub(pre.and_then(|c| c.system_cpu_usage).unwrap_or(0)) as f64;
    let cpus = cpu
        .and_then(|c| c.online_cpus)
        .or_else(|| {
            cpu.and_then(|c| c.cpu_usage.as_ref()?.percpu_usage.as_ref().map(|v| v.len() as u32))
        })
        .unwrap_or(1)
        .max(1) as f64;
    let cpu_pct =
        if sys_delta > 0.0 && cpu_delta > 0.0 { cpu_delta / sys_delta * cpus * 100.0 } else { 0.0 };

    let mem_raw = s.memory_stats.as_ref().and_then(|m| m.usage).unwrap_or(0);
    // Docker subtracts the page cache so the number matches `docker stats`.
    let cache = s
        .memory_stats
        .as_ref()
        .and_then(|m| m.stats.as_ref())
        .and_then(|st| st.get("inactive_file").or_else(|| st.get("cache")).copied())
        .unwrap_or(0);
    let mem = mem_raw.saturating_sub(cache);
    let mem_limit = s.memory_stats.as_ref().and_then(|m| m.limit).unwrap_or(0);

    let (rx, tx) = s
        .networks
        .as_ref()
        .map(|nets| {
            nets.values().fold((0u64, 0u64), |(r, t), n| {
                (r + n.rx_bytes.unwrap_or(0), t + n.tx_bytes.unwrap_or(0))
            })
        })
        .unwrap_or((0, 0));

    let (blk_r, blk_w) = s
        .blkio_stats
        .as_ref()
        .and_then(|b| b.io_service_bytes_recursive.as_ref())
        .map(|entries| {
            entries.iter().fold((0u64, 0u64), |(r, w), e| {
                let v = e.value.unwrap_or(0);
                match e.op.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    Some("read") => (r + v, w),
                    Some("write") => (r, w + v),
                    _ => (r, w),
                }
            })
        })
        .unwrap_or((0, 0));

    Sample {
        name,
        cpu: cpu_pct,
        mem,
        mem_limit,
        rx,
        tx,
        blk_r,
        blk_w,
        pids: s.pids_stats.as_ref().and_then(|p| p.current).unwrap_or(0),
    }
}

// ── rendering ───────────────────────────────────────────────────────────────

fn rgb(c: theme::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn load_color(pct: f64) -> Color {
    rgb(theme::load_color(pct))
}

/// Fractional bar in the theme's characters, drawn without ANSI so ratatui
/// can style it itself.
fn bar(pct: f64, width: usize) -> String {
    let g = theme::g();
    let filled = (pct.clamp(0.0, 100.0) / 100.0) * width as f64;
    let full = filled.floor() as usize;
    let rem = ((filled - full as f64) * 8.0).round() as usize;
    let mut s: String = g.bar_full.repeat(full.min(width));
    let mut used = full.min(width);
    if used < width && rem > 0 {
        s.push_str(g.bar_partials[rem - 1]);
        used += 1;
    }
    s.push_str(&g.bar_empty.repeat(width.saturating_sub(used)));
    s
}

fn draw(f: &mut ratatui::Frame, b: &Board, sort: Sort) {
    let [head, body, spark, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let total_cpu: f64 = b.rows.iter().map(|r| r.cpu).sum::<f64>().max(0.0);
    let total_mem: u64 = b.rows.iter().map(|r| r.mem).sum();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " dok stats ",
                Style::new().fg(rgb(theme::p().blue)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("· {} containers ", b.rows.len()),
                Style::new().fg(rgb(theme::p().gray)),
            ),
            Span::styled(format!("· cpu {total_cpu:.0}% "), Style::new().fg(load_color(total_cpu))),
            Span::styled(
                format!("· mem {} ", fmt::bytes(total_mem)),
                Style::new().fg(rgb(theme::p().cyan)),
            ),
            Span::styled(
                b.error.clone().map(|e| format!("· {e}")).unwrap_or_default(),
                Style::new().fg(rgb(theme::p().red)),
            ),
        ])),
        head,
    );

    let mut rows: Vec<&Sample> = b.rows.iter().collect();
    match sort {
        Sort::Cpu => rows.sort_by(|a, c| c.cpu.total_cmp(&a.cpu)),
        Sort::Mem => rows.sort_by_key(|r| std::cmp::Reverse(r.mem)),
        Sort::Name => rows.sort_by(|a, c| a.name.cmp(&c.name)),
    }

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|r| {
            let mem_pct =
                if r.mem_limit > 0 { r.mem as f64 / r.mem_limit as f64 * 100.0 } else { 0.0 };
            Row::new(vec![
                Cell::from(Span::styled(
                    r.name.clone(),
                    Style::new().fg(rgb(theme::hash_color(&r.name))).add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(bar(r.cpu, 16), Style::new().fg(load_color(r.cpu)))),
                Cell::from(Span::styled(
                    format!("{:>5.1}%", r.cpu),
                    Style::new().fg(load_color(r.cpu)),
                )),
                Cell::from(Span::styled(bar(mem_pct, 12), Style::new().fg(load_color(mem_pct)))),
                Cell::from(Span::styled(
                    format!("{:>8} / {}", fmt::bytes(r.mem), fmt::bytes(r.mem_limit)),
                    Style::new().fg(rgb(theme::p().fg)),
                )),
                Cell::from(Span::styled(
                    format!("↓{} ↑{}", fmt::bytes(r.rx), fmt::bytes(r.tx)),
                    Style::new().fg(rgb(theme::p().cyan)),
                )),
                Cell::from(Span::styled(
                    format!("r{} w{}", fmt::bytes(r.blk_r), fmt::bytes(r.blk_w)),
                    Style::new().fg(rgb(theme::p().magenta)),
                )),
                Cell::from(Span::styled(
                    format!("{:>4}", r.pids),
                    Style::new().fg(rgb(theme::p().gray)),
                )),
            ])
        })
        .collect();

    let header = Row::new(["CONTAINER", "CPU", "", "MEM", "USAGE", "NET", "BLOCK IO", "PIDS"])
        .style(Style::new().fg(rgb(theme::p().gray)).add_modifier(Modifier::BOLD));

    let widths = [
        Constraint::Min(14),
        Constraint::Length(16),
        Constraint::Length(6),
        Constraint::Length(12),
        Constraint::Length(20),
        Constraint::Length(18),
        Constraint::Length(18),
        Constraint::Length(5),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(rgb(theme::p().gray)))
        .padding(Padding::horizontal(1));

    if b.rows.is_empty() {
        let msg = if b.ticks == 0 { "sampling…" } else { "no running containers" };
        f.render_widget(
            Paragraph::new(Span::styled(msg, Style::new().fg(rgb(theme::p().gray)))).block(block),
            body,
        );
    } else {
        f.render_widget(Table::new(table_rows, widths).header(header).block(block), body);
    }

    f.render_widget(
        Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(rgb(theme::p().gray)))
                    .title(Span::styled(" total cpu % ", Style::new().fg(rgb(theme::p().gray)))),
            )
            .data(&b.total_cpu)
            .style(Style::new().fg(load_color(total_cpu))),
        spark,
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" q ", Style::new().fg(rgb(theme::p().blue))),
            Span::styled("quit  ", Style::new().fg(rgb(theme::p().gray))),
            Span::styled("s ", Style::new().fg(rgb(theme::p().blue))),
            Span::styled(format!("sort: {}", sort.label()), Style::new().fg(rgb(theme::p().gray))),
        ])),
        help,
    );
}
