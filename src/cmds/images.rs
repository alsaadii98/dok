//! `dok images` — images with size and age gradients.

use anyhow::Result;
use bollard::models::ImageSummary;
use clap::ValueEnum;

use crate::dk;
use crate::fmt;
use crate::table::{Column, Table};
use crate::theme::{self, *};

#[derive(Copy, Clone, ValueEnum)]
pub enum ImgSort {
    Size,
    Age,
    Name,
}

/// One row per repo:tag, since a single image id can carry several tags.
struct Entry<'a> {
    repo: String,
    tag: String,
    img: &'a ImageSummary,
    dangling: bool,
}

pub async fn run(all: bool, dangling_only: bool, sort: ImgSort) -> Result<()> {
    let docker = dk::connect()?;
    let list = dk::images(&docker, all).await?;

    let mut entries: Vec<Entry> = Vec::new();
    for img in &list {
        let tags: Vec<&String> =
            img.repo_tags.iter().filter(|t| t.as_str() != "<none>:<none>").collect();
        if tags.is_empty() {
            entries.push(Entry {
                repo: "<none>".into(),
                tag: "<none>".into(),
                img,
                dangling: true,
            });
        } else {
            for t in tags {
                let (prefix, name, tag) = fmt::split_image(t);
                entries.push(Entry { repo: format!("{prefix}{name}"), tag, img, dangling: false });
            }
        }
    }

    if dangling_only {
        entries.retain(|e| e.dangling);
    }

    if entries.is_empty() {
        println!("{}", dim("no images"));
        return Ok(());
    }

    match sort {
        ImgSort::Size => entries.sort_by_key(|e| -e.img.size),
        ImgSort::Age => entries.sort_by_key(|e| -e.img.created),
        ImgSort::Name => entries.sort_by(|a, b| (&a.repo, &a.tag).cmp(&(&b.repo, &b.tag))),
    }

    let mut t = Table::new(vec![
        Column::left(""),
        Column::left("REPOSITORY").flex(14).cap(34),
        Column::left("TAG").flex(6).cap(18),
        Column::left("ID"),
        Column::right("SIZE"),
        Column::right("AGE"),
        Column::left("USED BY"),
    ]);

    for e in &entries {
        t.row(render(e));
    }
    t.print();

    // Total counts unique image ids, not tags, so it matches `docker system df`.
    let mut ids: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    let total: i64 = list.iter().map(|i| i.size).sum();
    let dangling = entries.iter().filter(|e| e.dangling).count();
    let mut parts = vec![
        c(&format!("{} images", ids.len()), p().fg),
        c(&fmt::bytes(total.max(0) as u64), theme::size_color(total.max(0) as u64)),
    ];
    if dangling > 0 {
        parts.push(c(&format!("{dangling} dangling"), p().orange));
    }
    println!("\n{}", parts.join(&dim(" · ")));
    Ok(())
}

fn render(e: &Entry) -> Vec<String> {
    let full = format!("{}:{}", e.repo, e.tag);
    let icon = c(theme::image_icon(&full), if e.dangling { p().gray } else { p().magenta });

    let repo_cell = if e.dangling {
        c("<none>", p().gray)
    } else {
        match e.repo.rsplit_once('/') {
            Some((prefix, name)) => format!("{}{}", dim(&format!("{prefix}/")), c(name, p().fg)),
            None => c(&e.repo, p().fg),
        }
    };

    let tag_cell = if e.dangling {
        c("<none>", p().gray)
    } else if e.tag == "latest" {
        c(&e.tag, p().yellow)
    } else {
        c(&e.tag, p().cyan)
    };

    let size = e.img.size.max(0) as u64;
    let age = dk::age_secs(e.img.created);

    let used = match e.img.containers {
        n if n > 0 => c(&format!("{n} container{}", if n == 1 { "" } else { "s" }), p().green),
        _ if e.dangling => c("reclaimable", p().orange),
        _ => dim("—"),
    };

    vec![
        icon,
        repo_cell,
        tag_cell,
        dim(&fmt::short_id(&e.img.id)),
        c(&fmt::bytes(size), theme::size_color(size)),
        c(&fmt::age(age), theme::age_color(age)),
        used,
    ]
}
