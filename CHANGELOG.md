# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-08-18

### Added

- `dok update` — checks GitHub for a newer release and installs it. A
  standalone binary is replaced in place after its checksum is verified; a
  packaged one prints the right `brew` / `apk` / `dpkg` / `rpm` / `cargo`
  command instead. `--check` reports without installing.
- A once-a-day update check that prints one dim line after a command's output
  when a newer version exists. Off for piped output, `--demo`, and whenever
  `DOK_NO_UPDATE_CHECK` is set.

### Changed

- Tables no longer let one long value push every other column across the
  screen: `NAME`, `IMAGE`, `PORTS`, `REPOSITORY` and `TAG` are capped at a
  share of the terminal and truncated past it.
- Swarm container names lose their random task id — `api.1.hsnfrtha…` renders
  as `api.1`, which is the part that tells two replicas apart.
- Header underlines stop at the end of the word instead of running the full
  width of the column.

## [0.1.2] - 2026-08-17

### Fixed

- `dok ps | head` panicked with `failed printing to stdout: Broken pipe`.
  Rust ignores SIGPIPE by default; dok restores it, so piping into a reader
  that stops early now exits quietly like any other Unix tool.

### Added

- Alpine Linux packages. Every release now carries `dok-x86_64.apk` and
  `dok-aarch64.apk`, built with `abuild` inside Alpine from the same static
  musl archive the tarballs ship, and install-tested before publishing.
- `dok events --demo` replays a canned minute of daemon events, so the stream
  can be seen (and screenshotted) without a daemon.
- Animated SVG casts: `scripts/ansi2cast.py` renders captured output into an
  SVG that types the command, reveals the output and loops, with no JavaScript.
  The website hero cycles `ps`, `images`, `logs` and `events` in one file.

### Changed

- The static screenshots now use the same chrome and palette as the website's
  cards, so images and page are one surface instead of two.
- The site is a proper responsive layout: 6/4/1-column bento, terminal frames
  that scroll instead of shrinking, and a copy button on every install command.

## [0.1.1] - 2026-08-17

### Fixed

- `--demo` panicked with `SocketNotFoundError` on machines with no Docker
  socket, which was the one case it exists to serve. It now builds an offline
  client instead of probing the filesystem.
- `dok top --demo` printed a daemon error per container; it now has fixtures
  like every other demo command.

### Changed

- Homebrew installs a prebuilt binary instead of compiling from source, so it
  no longer pulls rust, llvm, z3 and python. `brew install --HEAD` still builds
  from source.

## [0.1.0] - 2026-08-17

First public release.

### Added

- `ps` (`ls`) — containers grouped by compose project, with short ids, state
  dots, health marks, `:8080→80` port mapping, compacted status and ages.
- `images` (`img`) — size and age gradients, dangling images marked
  reclaimable, per-image container counts.
- `df` (`du`) — disk usage per category with used/reclaimable bars, and
  `-v` for the biggest images, containers, volumes and build-cache entries.
- `inspect` — inspect JSON folded into identity/state/config/resources/
  network/mounts/labels, with credential-looking env values masked.
- `logs` — merged multi-container tail, stable per-container colours, stderr
  marked on the separator, level tokens and JSON lines highlighted.
- `top` — processes inside containers nested by parent PID.
- `tree` — compose projects, networks with container IPs, volumes with mount
  points.
- `stats` — live CPU/memory/IO dashboard with a total-CPU sparkline.
- `events` — colour-coded daemon event stream, `exec_*` hidden by default.
- `themes` — 10 built-in themes carrying palette, glyph set and layout, plus
  `--preview` and `--init`.
- Configuration via `~/.config/dok/config.toml`, including custom themes that
  inherit from a built-in.
- `--demo` on every command: renders a built-in example stack with no daemon,
  and is what generates the screenshots in the docs.

[Unreleased]: https://github.com/alsaadii98/cool-docker-commands/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/alsaadii98/cool-docker-commands/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/alsaadii98/cool-docker-commands/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/alsaadii98/cool-docker-commands/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/alsaadii98/cool-docker-commands/releases/tag/v0.1.0
