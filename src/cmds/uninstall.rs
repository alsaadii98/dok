//! `dok uninstall` — remove dok, or say exactly how to.
//!
//! dok only ever deletes what it owns. A package-manager install is reported,
//! never touched: the manager's own database has to stay right, and most of
//! those commands need a password dok has no business asking for. A standalone
//! binary is dok's to delete, and it deletes itself.

use anyhow::Result;

use crate::theme::{bold, c, dim, p};
use crate::uninstall::{self, Item};
use crate::update::{self, Install};

pub async fn run(purge: bool, yes: bool, dry_run: bool) -> Result<()> {
    // --demo exists so screenshots and casts can be recorded safely; a
    // recording session must never end with a deleted binary.
    let dry_run = dry_run || crate::demo::enabled();
    let install = update::detect();
    // Without --purge the binary goes and the settings stay, so a reinstall
    // finds the same themes and config it had before.
    let data = if purge { uninstall::data() } else { Vec::new() };

    let plan: Vec<Item> = match &install {
        Install::Standalone(exe) => {
            let mut v = vec![Item { what: "binary", path: exe.clone(), dir: false }];
            v.extend(uninstall::strays(exe));
            v.extend(data);
            v
        }
        _ => data,
    };

    if let Install::Standalone(exe) = &install
        && uninstall::looks_like_build_output(exe)
    {
        println!("{}", dim(&format!("{} is a build artefact, not an install", exe.display())));
        println!("{}", dim("nothing removed — run `cargo clean` instead"));
        return Ok(());
    }

    match &install {
        Install::Managed { by, remove, .. } => {
            println!("{} {}", dim("installed by"), bold(by));
            println!("{}", dim("dok does not remove packages it does not own; run:"));
            println!("  {}", bold(remove));
            if !purge {
                println!(
                    "\n{}",
                    dim("add --purge to also remove dok's config and cache directories")
                );
            }
        }
        Install::Unknown => {
            println!("{}", dim("cannot tell how dok was installed"));
            println!("{}", dim("delete the binary on your PATH by hand: `which dok`"));
        }
        Install::Standalone(_) => {}
    }

    if plan.is_empty() {
        if matches!(install, Install::Standalone(_)) {
            println!("{}", dim("nothing to remove"));
        }
        return Ok(());
    }

    println!();
    for item in &plan {
        println!("{} {}", dim(&format!("{:>7}", item.what)), item.path.display());
    }

    if dry_run {
        println!("\n{}", dim("--dry-run: nothing removed"));
        return Ok(());
    }
    if !yes && !confirm(&format!("remove {} item(s)?", plan.len())) {
        println!("{}", dim("nothing changed"));
        return Ok(());
    }

    // The binary goes last. If a data directory cannot be removed the user
    // still has a working dok to try again with.
    let (bin, rest): (Vec<&Item>, Vec<&Item>) =
        plan.iter().partition(|i| i.what == "binary" || i.what == "shim");
    let mut failed = 0usize;
    for item in rest.into_iter().chain(bin) {
        let r = if item.what == "binary" {
            uninstall::remove_self(&item.path)
        } else {
            uninstall::remove(item)
        };
        match r {
            Ok(()) => {
                println!("{} {}", c("removed", p().green), dim(&item.path.display().to_string()))
            }
            Err(e) => {
                failed += 1;
                eprintln!("{} {e:#}", c("failed", p().red));
            }
        }
    }

    // Only a removed binary means dok is really gone; a managed install has
    // just had its settings cleared and still needs its manager's command.
    if failed == 0 {
        if matches!(install, Install::Standalone(_)) {
            println!("\n{}", dim("dok is gone. thanks for trying it."));
        } else {
            println!("\n{}", dim("config and cache removed; the binary is still installed."));
        }
    }
    Ok(())
}

/// A y/n prompt that answers itself with "no" when there is nobody to ask.
fn confirm(question: &str) -> bool {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        println!("{}", dim("not a terminal — rerun with --yes to remove"));
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim(), "y" | "Y" | "yes")
}
