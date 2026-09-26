//! `dok themes` — list what is installed and preview how each one looks.

use anyhow::Result;

use crate::config::{self, Config};
use crate::fmt;
use crate::json;
use crate::theme::{self, Theme, builtin};

pub async fn run(cfg: &Config, active: &str, preview: bool) -> Result<()> {
    let names = config::theme_names(cfg);

    if json::enabled() {
        let hex = |c: theme::Rgb| format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2);
        let mut out = Vec::new();
        for name in &names {
            let t = config::resolve_theme(cfg, name)?;
            let p = &t.palette;
            out.push(serde_json::json!({
                "name": name,
                "description": t.description,
                "active": name == active,
                // The nine semantic roles, resolved — a consumer can rebuild
                // dok's colouring without knowing which theme produced it.
                "palette": {
                    "green": hex(p.green), "red": hex(p.red), "yellow": hex(p.yellow),
                    "orange": hex(p.orange), "blue": hex(p.blue), "cyan": hex(p.cyan),
                    "magenta": hex(p.magenta), "gray": hex(p.gray), "fg": hex(p.fg),
                },
            }));
        }
        json::emit(&serde_json::json!({ "themes": out, "active": active }));
        return Ok(());
    }

    if !preview {
        println!("{}", theme::header(&fmt::pad("  THEME", 18)));
        for name in &names {
            let t = config::resolve_theme(cfg, name)?;
            let mark = if *name == active { theme::c("●", t.palette.green) } else { " ".into() };
            println!(
                "{mark} {} {}  {}",
                theme::cb(&fmt::pad(name, 16), t.palette.blue),
                swatch(&t),
                theme::dim(t.description)
            );
        }
        println!(
            "\n{}",
            theme::dim("dok themes --preview shows a full sample · --theme <name> to use one")
        );
        return Ok(());
    }

    for name in &names {
        let t = config::resolve_theme(cfg, name)?;
        sample(&t, *name == active);
        println!();
    }
    Ok(())
}

/// Nine blocks, one per palette role.
fn swatch(t: &Theme) -> String {
    let p = &t.palette;
    [p.green, p.cyan, p.blue, p.magenta, p.yellow, p.orange, p.red, p.fg, p.gray]
        .iter()
        .map(|c| theme::c(t.chars.bar_full, *c))
        .collect::<Vec<_>>()
        .join("")
}

/// A miniature `ps` rendered in the given theme, without touching the daemon.
fn sample(t: &Theme, active: bool) {
    let p = &t.palette;
    let g = &t.chars;

    let title = if active {
        format!("{} {}", theme::cb(&t.name, p.blue), theme::c("(active)", p.green))
    } else {
        theme::cb(&t.name, p.blue)
    };
    println!("{title}  {}  {}", swatch(t), theme::dim(t.description));

    // Header, drawn with this theme's rules rather than the active one.
    let cols = [("", 2), ("NAME", 12), ("IMAGE", 20), ("STATUS", 12), ("CPU", 14)];
    let head: Vec<String> =
        cols.iter().map(|(title, w)| header_in(t, &fmt::pad(title, *w))).collect();
    // Mirror the real table joiner so grid themes preview accurately.
    let gutter = match t.layout.column_sep {
        Some(sep) => format!(
            "{}{}{}",
            " ".repeat(t.layout.gutter.div_ceil(2)),
            theme::c(sep, p.gray),
            " ".repeat(t.layout.gutter / 2)
        ),
        None => " ".repeat(t.layout.gutter),
    };
    println!("{}", head.join(&gutter));
    if let Some(rule) = t.layout.rule {
        let span: usize = cols.iter().map(|(_, w)| w).sum::<usize>() + t.layout.gutter * 4;
        println!("{}", theme::c(&rule.repeat(span), p.gray));
    }

    let rows: [(&str, &str, &str, &str, f64); 3] = [
        (g.dot_running, "api", "node:20-alpine", "up 3h", 12.0),
        (g.dot_running, "postgres", "postgres:16", "up 3h", 63.0),
        (g.dot_stopped, "worker", "ghcr.io/acme/w", "exited (137)", 0.0),
    ];
    for (i, (dot, name, image, status, cpu)) in rows.iter().enumerate() {
        let stub = if i + 1 == rows.len() { g.tree_last } else { g.tree_branch };
        let running = *cpu > 0.0;
        let dot_col = if running { p.green } else { p.gray };
        let status_col = if status.starts_with("exited") { p.red } else { p.green };
        println!(
            "{}{}{}{}{}{}{}{}{}",
            theme::c(&fmt::pad(dot, 2), dot_col),
            gutter,
            fmt::pad(
                &format!(
                    "{}{}",
                    theme::c(stub, p.gray),
                    theme::cb(name, if running { p.fg } else { p.gray })
                ),
                12
            ),
            gutter,
            fmt::pad(&theme::c(image, p.cyan), 20),
            gutter,
            fmt::pad(&theme::c(status, status_col), 12),
            gutter,
            bar_in(t, *cpu, 12),
        );
    }
}

/// Header and bar renderers that use `t` instead of the globally active theme.
fn header_in(t: &Theme, s: &str) -> String {
    if !theme::color_enabled() {
        return s.to_string();
    }
    let text = match t.layout.header {
        theme::HeaderStyle::Dim => s.to_lowercase(),
        theme::HeaderStyle::Caps => s.to_uppercase(),
        _ => s.to_string(),
    };
    let sgr = match t.layout.header {
        theme::HeaderStyle::Underline => "1;4;",
        theme::HeaderStyle::Bold | theme::HeaderStyle::Caps => "1;",
        theme::HeaderStyle::Dim => "",
    };
    let g = t.palette.gray;
    format!("\x1b[{sgr}38;2;{};{};{}m{text}\x1b[0m", g.0, g.1, g.2)
}

fn bar_in(t: &Theme, pct: f64, width: usize) -> String {
    let g = &t.chars;
    let filled = (pct.clamp(0.0, 100.0) / 100.0) * width as f64;
    let full = filled.floor() as usize;
    let rem = ((filled - full as f64) * 8.0).round() as usize;
    let mut s = g.bar_full.repeat(full.min(width));
    let mut used = full.min(width);
    if used < width && rem > 0 {
        s.push_str(g.bar_partials[rem - 1]);
        used += 1;
    }
    let col = match pct {
        v if v < 25.0 => t.palette.green,
        v if v < 60.0 => t.palette.cyan,
        v if v < 85.0 => t.palette.yellow,
        _ => t.palette.red,
    };
    format!(
        "{}{}",
        theme::c(&s, col),
        theme::c(&g.bar_empty.repeat(width.saturating_sub(used)), t.palette.gray)
    )
}

/// Write a starter config the user can edit.
pub fn write_starter_config() -> Result<()> {
    let Some(path) = config::config_path() else {
        anyhow::bail!("cannot locate a config directory (set XDG_CONFIG_HOME or HOME)")
    };
    if path.exists() {
        println!("{} {}", theme::dim("config already exists at"), path.display());
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let names: Vec<String> = builtin::all().iter().map(|t| t.name.clone()).collect();
    let body = format!(
        "# dok configuration\n\
         # themes: {}\n\
         theme = \"default\"\n\
         # icons = \"nerd\"      # auto | nerd | unicode | none\n\n\
         # A custom theme starts from a built-in and overrides what it wants.\n\
         # [themes.mine]\n\
         # base = \"gruvbox\"\n\
         # glyphs = \"heavy\"    # unicode | ascii | heavy | slim\n\
         # layout = \"grid\"     # default | ruled | grid | quiet | ascii\n\
         # header = \"caps\"     # underline | bold | dim | caps\n\
         # gutter = 2\n\
         # green = \"#b8bb26\"\n",
        names.join(", ")
    );
    std::fs::write(&path, body)?;
    println!("{} {}", theme::dim("wrote"), path.display());
    Ok(())
}
