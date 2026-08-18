//! `dok update` — check for a newer release and install it.

use anyhow::Result;

use crate::theme::{bold, c, dim, p};
use crate::update::{self, Install};

pub async fn run(check_only: bool, yes: bool) -> Result<()> {
    let current = update::current();
    println!("{} {}", dim("installed"), bold(current));

    let latest = update::latest_version(10)?;
    update::write_cache(&latest);

    if !update::is_newer(&latest, current) {
        println!("{}", c("already up to date", p().green));
        return Ok(());
    }

    println!(
        "{} {}   {}",
        dim("available"),
        c(&latest, p().green),
        dim(&format!("https://github.com/{}/releases/tag/v{latest}", update::REPO))
    );

    if check_only {
        return Ok(());
    }

    match update::detect() {
        Install::Managed { by, cmd } => {
            println!("\n{}", dim(&format!("dok was installed by {by}; update it with:")));
            println!("  {}", bold(&cmd));
        }
        Install::Unknown => {
            println!(
                "\n{}",
                dim("cannot tell how dok was installed — grab the archive from the release page")
            );
        }
        Install::Standalone(exe) => {
            if !yes && !confirm(&format!("replace {} with {latest}?", exe.display())) {
                println!("{}", dim("nothing changed"));
                return Ok(());
            }
            println!("{}", dim("downloading…"));
            update::replace(&exe, &latest)?;
            println!("{} {}", c("updated to", p().green), bold(&latest));
        }
    }
    Ok(())
}

/// A y/n prompt that answers itself with "no" when there is nobody to ask.
fn confirm(question: &str) -> bool {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        println!("{}", dim("not a terminal — rerun with --yes to install"));
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
