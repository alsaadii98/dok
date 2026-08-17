//! dok — docker output, made readable.

mod cmds;
mod config;
mod demo;
mod dk;
mod fmt;
mod table;
mod theme;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "dok",
    version,
    about = "Docker output, made readable",
    long_about = "dok renders docker's output the way eza renders ls: colour, icons, \
                  human sizes and ages, grouped by compose project."
)]
struct Cli {
    /// When to colourise output
    #[arg(long, value_enum, default_value = "auto", global = true)]
    color: ColorChoice,

    /// Which icon set to use
    #[arg(long, value_enum, default_value = "auto", global = true)]
    icons: IconChoice,

    /// Theme name (see `dok themes`); overrides DOK_THEME and the config file
    #[arg(long, global = true)]
    theme: Option<String>,

    /// Render a canned example stack instead of talking to a daemon
    #[arg(long, global = true, hide = true)]
    demo: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Copy, Clone, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, ValueEnum)]
enum IconChoice {
    /// Nerd Font glyphs when the terminal looks capable, else unicode
    Auto,
    /// Nerd Font glyphs (requires a patched font)
    Nerd,
    /// Plain unicode symbols
    Unicode,
    /// No icons at all
    None,
}

#[derive(Subcommand)]
enum Cmd {
    /// List containers, grouped by compose project
    #[command(visible_alias = "ls")]
    Ps {
        /// Include stopped containers
        #[arg(short, long, action = ArgAction::SetTrue)]
        all: bool,
        /// Do not group by compose project
        #[arg(long, action = ArgAction::SetTrue)]
        flat: bool,
        /// Only containers whose name or image matches
        #[arg(short, long)]
        filter: Option<String>,
        /// Sort key
        #[arg(short, long, value_enum, default_value = "name")]
        sort: cmds::ps::PsSort,
    },

    /// List images with size and age gradients
    #[command(visible_alias = "img")]
    Images {
        /// Include intermediate layers
        #[arg(short, long, action = ArgAction::SetTrue)]
        all: bool,
        /// Only dangling (<none>) images
        #[arg(long, action = ArgAction::SetTrue)]
        dangling: bool,
        /// Sort key
        #[arg(short, long, value_enum, default_value = "size")]
        sort: cmds::images::ImgSort,
    },

    /// Tail logs from one or more containers with level colouring
    Logs {
        /// Container names or ids; omit to tail every running container
        containers: Vec<String>,
        /// Keep streaming
        #[arg(short, long, action = ArgAction::SetTrue)]
        follow: bool,
        /// Lines of history per container
        #[arg(short = 'n', long, default_value = "40")]
        tail: String,
        /// Show container timestamps
        #[arg(short, long, action = ArgAction::SetTrue)]
        timestamps: bool,
        /// Only lines containing this substring
        #[arg(short, long)]
        grep: Option<String>,
    },

    /// Live CPU / memory / IO dashboard
    Stats {
        /// Limit to these containers
        containers: Vec<String>,
        /// Refresh interval in milliseconds
        #[arg(long, default_value = "1500")]
        interval: u64,
    },

    /// Disk usage by images, containers, volumes and build cache
    #[command(visible_alias = "du")]
    Df {
        /// Also list the biggest items in each category
        #[arg(short, long, action = ArgAction::SetTrue)]
        verbose: bool,
        /// How many items to list per category with --verbose
        #[arg(long, default_value = "10")]
        top: usize,
    },

    /// Processes running inside containers, as a tree
    Top {
        /// Containers to inspect; omit for every running container
        containers: Vec<String>,
        /// Arguments passed to ps inside the container
        #[arg(long)]
        ps_args: Option<String>,
        /// Do not nest child processes under their parent
        #[arg(long, action = ArgAction::SetTrue)]
        flat: bool,
    },

    /// Readable summary of a container's configuration and state
    Inspect {
        /// Container names or ids
        #[arg(required = true)]
        containers: Vec<String>,
        /// Print environment variables
        #[arg(short, long, action = ArgAction::SetTrue)]
        env: bool,
        /// Do not mask password/token-looking env values
        #[arg(long, action = ArgAction::SetTrue)]
        show_secrets: bool,
    },

    /// Live daemon event stream, colour-coded
    Events {
        /// Show events created since this timestamp or duration (e.g. 1h)
        #[arg(long)]
        since: Option<String>,
        /// Stop at this timestamp instead of streaming forever
        #[arg(long)]
        until: Option<String>,
        /// Restrict to these object types (container, image, volume, network, …)
        #[arg(short = 'T', long, value_delimiter = ',')]
        r#type: Vec<String>,
        /// Only lines containing this substring
        #[arg(short, long)]
        grep: Option<String>,
        /// Include exec_* events (docker exec and healthcheck probes)
        #[arg(long, action = ArgAction::SetTrue)]
        exec: bool,
    },

    /// List or preview themes
    Themes {
        /// Render a full sample of every theme
        #[arg(short, long, action = ArgAction::SetTrue)]
        preview: bool,
        /// Write a starter config file to ~/.config/dok/config.toml
        #[arg(long, action = ArgAction::SetTrue)]
        init: bool,
    },

    /// Tree view of compose projects, networks and volumes
    Tree {
        /// Show only this section
        #[arg(long, value_enum)]
        only: Option<cmds::tree::Section>,
        /// Include stopped containers
        #[arg(short, long, action = ArgAction::SetTrue)]
        all: bool,
    },
}

/// Rust ignores SIGPIPE and turns the resulting EPIPE into a panic, so
/// `dok ps | head` dies with a backtrace instead of stopping quietly. Every
/// command here is meant to be piped, so restore the default disposition.
#[cfg(unix)]
fn restore_sigpipe() {
    // SAFETY: setting a signal disposition to SIG_DFL before any threads that
    // could care exist; this is the documented fix for the EPIPE panic.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe() {}

#[tokio::main]
async fn main() -> Result<()> {
    restore_sigpipe();

    let cli = Cli::parse();

    demo::set(cli.demo || std::env::var_os("DOK_DEMO").is_some());

    let cfg = config::load()?;

    // Precedence: --theme, then DOK_THEME, then the config file, then default.
    let theme_name = cli
        .theme
        .clone()
        .or_else(|| std::env::var("DOK_THEME").ok().filter(|s| !s.is_empty()))
        .or_else(|| cfg.theme.clone())
        .unwrap_or_else(|| "default".to_string());
    theme::set_theme(config::resolve_theme(&cfg, &theme_name)?);

    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let no_color_env = std::env::var_os("NO_COLOR").is_some();
    theme::set_color(match cli.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => is_tty && !no_color_env,
    });
    // The config can set icons too; the flag still wins when given explicitly.
    let icon_choice = match (cli.icons, cfg.icons.as_deref()) {
        (IconChoice::Auto, Some("nerd")) => IconChoice::Nerd,
        (IconChoice::Auto, Some("unicode")) => IconChoice::Unicode,
        (IconChoice::Auto, Some("none")) => IconChoice::None,
        (other, _) => other,
    };
    theme::set_icons(match icon_choice {
        IconChoice::Nerd => theme::IconSet::Nerd,
        IconChoice::Unicode => theme::IconSet::Unicode,
        IconChoice::None => theme::IconSet::None,
        IconChoice::Auto => {
            if !is_tty {
                theme::IconSet::None
            } else if nerd_font_likely() {
                theme::IconSet::Nerd
            } else {
                theme::IconSet::Unicode
            }
        }
    });

    match cli.cmd {
        Cmd::Ps { all, flat, filter, sort } => cmds::ps::run(all, flat, filter, sort).await,
        Cmd::Images { all, dangling, sort } => cmds::images::run(all, dangling, sort).await,
        Cmd::Logs { containers, follow, tail, timestamps, grep } => {
            cmds::logs::run(containers, follow, tail, timestamps, grep).await
        }
        Cmd::Stats { containers, interval } => cmds::stats::run(containers, interval).await,
        Cmd::Df { verbose, top } => cmds::df::run(verbose, top).await,
        Cmd::Top { containers, ps_args, flat } => cmds::top::run(containers, ps_args, flat).await,
        Cmd::Inspect { containers, env, show_secrets } => {
            cmds::inspect::run(containers, show_secrets, env).await
        }
        Cmd::Events { since, until, r#type, grep, exec } => {
            cmds::events::run(since, until, r#type, grep, exec).await
        }
        Cmd::Themes { preview, init } => {
            if init {
                cmds::themes::write_starter_config()
            } else {
                cmds::themes::run(&cfg, &theme_name, preview).await
            }
        }
        Cmd::Tree { only, all } => cmds::tree::run(only, all).await,
    }
}

/// Guess whether the terminal is running a patched font. Opt-in env var wins.
fn nerd_font_likely() -> bool {
    if let Ok(v) = std::env::var("DOK_NERD_FONT") {
        return v != "0" && !v.is_empty();
    }
    // These terminals ship or are commonly paired with patched fonts.
    matches!(std::env::var("TERM_PROGRAM").as_deref(), Ok("WezTerm") | Ok("ghostty") | Ok("kitty"))
}
