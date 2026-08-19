# Contributing to dok

Thanks for looking. Bug reports, theme submissions and new commands are all
welcome. This is a small codebase on purpose — read it in an afternoon, change
it in an hour.

## Getting set up

You need Rust 1.88+ (edition 2024) and a running Docker daemon.

```sh
git clone https://github.com/alsaadii98/cool-docker-commands
cd cool-docker-commands
cargo build
./target/debug/dok ps -a
```

Before opening a PR:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs exactly those three on Linux and macOS.

## How the code is laid out

```
src/
  main.rs          CLI definition (clap) and startup: theme, colour, icons
  config.rs        ~/.config/dok/config.toml, custom theme resolution
  dk.rs            Docker daemon access + small helpers over bollard's models
  fmt.rs           humanizers: bytes, ages, ANSI-aware widths, truncation
  table.rs         column layout engine — measures, shrinks to fit, prints
  theme.rs         palette / glyphs / layout, painting helpers
  theme/builtin.rs the built-in themes
  cmds/*.rs        one file per subcommand
  demo.rs          canned fixtures behind --demo, used by the docs
scripts/
  ansi2svg.py      renders real command output to the SVGs used in the docs
docs/              website (GitHub Pages) and generated screenshots
packaging/         homebrew, AUR, alpine
bucket/            scoop manifest (this repo doubles as a scoop bucket)
```

Two rules keep the output consistent:

1. **Never name a colour, name a role.** Use `p().green`, `p().gray`, and the
   gradient helpers (`theme::size_color`, `theme::age_color`,
   `theme::load_color`). A command that hardcodes an RGB value breaks every
   theme.
2. **Never hardcode a structural glyph.** Tree stubs, bars, arrows, separators
   and state dots all come from `g()`, so the `ascii` theme stays ASCII.

Cells are pre-coloured strings, so all width maths must go through
`fmt::visible_width`, `fmt::pad`, `fmt::rpad` and `fmt::truncate` — plain
`str::len()` counts escape bytes and will misalign the table.

## Adding a command

1. Create `src/cmds/<name>.rs` with `pub async fn run(...) -> anyhow::Result<()>`.
2. Register it in `src/cmds/mod.rs`.
3. Add the variant to `enum Cmd` in `src/main.rs` and dispatch it in `main`.
4. Build the output with `table::Table` (or the tree helpers in `cmds/tree.rs`).
5. Add a row to the command table in `README.md`.

## Adding a theme

Themes live in `src/theme/builtin.rs`. A theme is a palette, a glyph set and a
layout — reuse the existing `UNICODE` / `ASCII` / `HEAVY` / `SLIM` glyph sets
and `LAYOUT_*` layouts unless your theme genuinely needs new ones.

```rust
theme("mytheme", "one-line description", MY_PALETTE, UNICODE, LAYOUT_DEFAULT),
```

Check it in both directions before submitting:

```sh
cargo run -- themes --preview
cargo run -- ps -a --theme mytheme
```

Palette guidance: `gray` must stay readable on your target background (it
carries all secondary text), and `red` must be distinguishable from `orange`
for people with red-green colour blindness — the `df` bars put them adjacent.

## Regenerating the screenshots

The images in `README.md` and the website are generated from real output, so
they cannot drift:

```sh
./scripts/gen-screenshots.sh
```

Run it with a couple of containers up, ideally from a compose project, so the
grouping is visible.

## Commit and PR conventions

- Conventional-ish subjects: `feat: add dok ports`, `fix: align grid gutters`,
  `docs: …`, `chore: …`.
- One logical change per PR. A new command and a refactor of the table engine
  are two PRs.
- If you change output, paste a before/after in the PR description.

## Releasing (maintainers)

```sh
# bump version in Cargo.toml, update CHANGELOG.md, then:
git tag -a v0.2.0 -m "v0.2.0"
git push origin v0.2.0
```

The release workflow builds every target, attaches archives, `.deb` and `.rpm`
with checksums, publishes to crates.io and opens the Homebrew tap bump.

## Code of conduct

Be decent. Assume good faith, keep criticism about the code, and no
harassment — maintainers will remove comments and contributors that make the
project worse to be around.
