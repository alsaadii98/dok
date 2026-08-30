//! `dok history` — the layers of an image, with size bars and readable instructions.
//!
//! `docker history` prints the right data in the wrong order and hides the
//! instruction behind `/bin/sh -c #(nop)`. This reads top-down in build order,
//! the way the Dockerfile was written, and marks the layers worth deleting.

use anyhow::Result;
use bollard::models::ImageHistoryResponseItem;

use crate::dk;
use crate::fmt;
use crate::table::{Column, Table};
use crate::theme::{self, *};

/// A layer big enough to be worth arguing about, as a share of the image.
const FAT_SHARE: f64 = 0.10;

pub async fn run(image: String, reverse: bool, no_trunc: bool) -> Result<()> {
    let docker = dk::connect()?;
    let name = dk::resolve_image(&docker, &image).await?;
    let mut layers = dk::image_history(&docker, &name).await?;

    if layers.is_empty() {
        println!("{}", dim("no layers"));
        return Ok(());
    }

    // The daemon answers newest-first. A Dockerfile is read top-down, and so is
    // this, unless someone asks for docker's order back.
    if !reverse {
        layers.reverse();
    }

    let total: i64 = layers.iter().map(|l| l.size.max(0)).sum();

    let mut t = Table::new(vec![
        Column::left(""),
        Column::right("#"),
        Column::right("SIZE"),
        Column::left(""),
        Column::right("AGE"),
        Column::left("INSTRUCTION").flex(20),
    ]);

    for (i, l) in layers.iter().enumerate() {
        let n = if reverse { layers.len() - i } else { i + 1 };
        t.row(render(l, n, total, no_trunc));
    }
    t.print();
    println!("{}", summary(&name, &layers, total));
    Ok(())
}

fn render(l: &ImageHistoryResponseItem, n: usize, total: i64, no_trunc: bool) -> Vec<String> {
    let size = l.size.max(0) as u64;
    let share = if total > 0 { size as f64 / total as f64 } else { 0.0 };

    // Only the layers actually worth looking at get a mark.
    let mark = if share >= FAT_SHARE { c(g().bullet, p().orange) } else { String::new() };

    // A zero-byte layer is metadata — ENV, CMD, LABEL. Real, but not the
    // reason an image is large, so it stays quiet.
    let size_cell =
        if size == 0 { dim(&fmt::bytes(0)) } else { c(&fmt::bytes(size), theme::size_color(size)) };

    let instruction = clean_instruction(&l.created_by);
    let instruction = if no_trunc { instruction } else { instruction.replace('\n', " ") };
    let (verb, rest) = split_verb(&instruction);
    let text = if rest.is_empty() {
        cb(&verb, p().cyan)
    } else {
        format!("{} {}", cb(&verb, p().cyan), c(&rest, p().fg))
    };

    let age = dk::age_secs(l.created);
    vec![
        mark,
        dim(&n.to_string()),
        size_cell,
        share_bar(share),
        c(&fmt::age(age), theme::age_color(age)),
        text,
    ]
}

/// `████░░░░` — this layer's share of the whole image.
fn share_bar(share: f64) -> String {
    const WIDTH: usize = 12;
    if share <= 0.0 {
        return dim(&g().bar_empty.repeat(WIDTH));
    }
    let cells = ((share * WIDTH as f64).round() as usize).clamp(1, WIDTH);
    let col = if share >= FAT_SHARE { p().orange } else { p().green };
    format!("{}{}", c(&g().bar_full.repeat(cells), col), dim(&g().bar_empty.repeat(WIDTH - cells)))
}

/// Recover the Dockerfile instruction from what the daemon recorded.
///
/// Classic builder writes `/bin/sh -c #(nop)  CMD ["x"]` for metadata and
/// `/bin/sh -c <command>` for a RUN. BuildKit writes the instruction directly.
/// All three should read as the line someone actually wrote.
fn clean_instruction(raw: &str) -> String {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("/bin/sh -c #(nop)") {
        return collapse(rest);
    }
    if let Some(rest) = s.strip_prefix("/bin/sh -c ") {
        return format!("RUN {}", collapse(rest));
    }
    // BuildKit prefixes its own steps; the instruction is what follows.
    if let Some(rest) = s.strip_prefix("RUN /bin/sh -c ") {
        return format!("RUN {}", collapse(rest));
    }
    collapse(s)
}

/// Squeeze runs of whitespace so a multi-line RUN fits one row.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split `RUN apt-get update` into the verb and the rest, so the verb can be
/// coloured the way a keyword should be.
fn split_verb(s: &str) -> (String, String) {
    const VERBS: [&str; 14] = [
        "ADD",
        "ARG",
        "CMD",
        "COPY",
        "ENTRYPOINT",
        "ENV",
        "EXPOSE",
        "HEALTHCHECK",
        "LABEL",
        "RUN",
        "SHELL",
        "USER",
        "VOLUME",
        "WORKDIR",
    ];
    match s.split_once(' ') {
        Some((head, rest)) if VERBS.contains(&head) => (head.to_string(), rest.to_string()),
        _ if VERBS.contains(&s) => (s.to_string(), String::new()),
        _ => (String::new(), s.to_string()),
    }
}

fn summary(name: &str, layers: &[ImageHistoryResponseItem], total: i64) -> String {
    let sized = layers.iter().filter(|l| l.size > 0).count();
    let empty = layers.len() - sized;
    let fat = layers
        .iter()
        .filter(|l| total > 0 && (l.size.max(0) as f64 / total as f64) >= FAT_SHARE)
        .count();

    let mut parts = vec![
        c(name, p().magenta),
        c(&format!("{} layers", layers.len()), p().fg),
        c(&fmt::bytes(total.max(0) as u64), theme::size_color(total.max(0) as u64)),
    ];
    if empty > 0 {
        parts.push(dim(&format!("{empty} metadata")));
    }
    if fat > 0 {
        parts.push(c(&format!("{fat} over {:.0}%", FAT_SHARE * 100.0), p().orange));
    }
    format!("\n{}", parts.join(&dim(" · ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nop_metadata_layers_lose_their_shell_wrapper() {
        assert_eq!(
            clean_instruction(r#"/bin/sh -c #(nop)  CMD ["nginx" "-g" "daemon off;"]"#),
            r#"CMD ["nginx" "-g" "daemon off;"]"#
        );
        assert_eq!(clean_instruction("/bin/sh -c #(nop) WORKDIR /app"), "WORKDIR /app");
    }

    #[test]
    fn a_shell_layer_reads_as_the_run_it_was() {
        assert_eq!(clean_instruction("/bin/sh -c apt-get update"), "RUN apt-get update");
        assert_eq!(clean_instruction("RUN /bin/sh -c make build"), "RUN make build");
    }

    /// BuildKit records the instruction directly; leave it alone.
    #[test]
    fn buildkit_instructions_pass_through() {
        assert_eq!(clean_instruction("COPY . /app"), "COPY . /app");
    }

    /// A RUN spanning several lines has to fit one row.
    #[test]
    fn multi_line_commands_collapse_to_one_line() {
        assert_eq!(
            clean_instruction("/bin/sh -c set -eux;   \n  apt-get update;  \n  rm -rf /var"),
            "RUN set -eux; apt-get update; rm -rf /var"
        );
    }

    #[test]
    fn the_verb_splits_off_for_colouring() {
        assert_eq!(split_verb("RUN apt-get update"), ("RUN".into(), "apt-get update".into()));
        assert_eq!(split_verb("WORKDIR"), ("WORKDIR".into(), String::new()));
        // Not a Dockerfile verb: leave the whole string as the body.
        assert_eq!(split_verb("some raw thing"), (String::new(), "some raw thing".into()));
    }

    #[test]
    fn the_bar_never_hides_a_layer_that_has_bytes() {
        // A layer far under one cell still gets one, or it reads as empty.
        assert!(share_bar(0.001).contains(g().bar_full));
        assert!(!share_bar(0.0).contains(g().bar_full));
    }
}
