# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `--json` on every command that has something to show. It carries dok's data
  rather than docker's raw API — compose project, service name, health verdict,
  reclaimable flag — because those derived fields are the reason to use dok at
  all. Sizes and ages come as both the number and the rendering
  (`{"bytes": 1181116006, "human": "1.1GB"}`), so nothing has to be re-parsed.
  `dok logs` and `dok events` are streams and emit NDJSON, one object per line,
  flushed as it arrives. `--json` implies `--color never` and suppresses the
  update notice, since anything appended after the document breaks every parser.
  `dok stats` is a live dashboard and refuses with a non-zero exit instead.
  `dok inspect --json` masks exactly what the table masks: no `env` without
  `--env`, and credential-looking values replaced by their length without
  `--show-secrets`.

## [0.2.1] - 2026-09-21

### Changed

- `sha2` 0.10 → 0.11, the crate that verifies release checksums before `dok
  update` replaces the binary. The digest is now pinned by a test against the
  published SHA-256 reference vector, so a future bump cannot change it quietly.

### Fixed

- Truncating a coloured cell could cut an ANSI escape sequence in half,
  leaving a bare ESC byte in the output. A terminal swallows that quietly;
  an SVG does not, which is how it surfaced — `cast-health.svg` was not
  well-formed XML and the site showed a broken image. `fmt::truncate` now
  copies escape sequences through whole and cost-free, the way
  `visible_width` already counted them, and closes any open colour before
  the ellipsis so it no longer bleeds into the gutter.

- `dok inspect api --demo` reported the container's name as `api` — whatever
  was typed — instead of `demo-shop-api-1`. A regression from 0.1.6, when the
  demo fixture started varying by service. The fixture now resolves the way
  a real daemon does: by container name, service name or id prefix.

## [0.2.0] - 2026-09-11

### Added

- `dok completions <shell>` — prints a completion script for bash, zsh, fish,
  nushell, powershell or elvish, generated from the same clap definition the
  CLI is built from, so it cannot drift. The update notice is suppressed for
  this command, since the output is sourced by a shell. Homebrew installs
  completions by running the binary at install time; the `.deb` and `.rpm`
  carry bash, zsh and fish in the distro-standard locations; and every release
  attaches a `dok-<version>-completions.tar.gz` with all six.

- `dok prune` — what `docker system prune` would remove, grouped by kind and
  sized, before you run it. Never deletes anything; it prints the docker
  command that does. Draws the same line docker draws: dangling images by
  default, every unused image with `-a`, volumes only with `--volumes`.
  Networks are judged by cross-referencing every container's own settings,
  since the list endpoint does not report membership, and the three built-in
  networks docker refuses to remove are never listed.

### Fixed

- The demo stack attached every container to `demo-shop_default`, including
  grafana, whose IP is in the observability subnet. `dok tree --demo` showed
  `observability_default` with no containers. Networks now follow the compose
  project. The stack also gains a leftover `old-release_default` network and a
  tagged-but-unused `api:1.4.1` image, so `dok prune --demo` has something to
  show on each side of the `-a` boundary.

## [0.1.6] - 2026-08-27

### Added

- `dok history <image>` — the layers of an image with a size bar each, read
  top-down in build order the way the Dockerfile was written rather than
  newest-first the way `docker history` prints it. Recovers the instruction
  from both the classic builder's `/bin/sh -c #(nop)` wrapper and BuildKit's
  direct form, collapses a multi-line `RUN` onto one row, and marks every layer
  over 10% of the image. `-r` restores docker's order, `--no-trunc` keeps long
  instructions whole.
- `dok health` — the containers that declare a healthcheck, and only those,
  with the failing streak, the first line of the last probe's output and the
  probe's exit code. Unhealthy sorts first. `-u` narrows to what is failing,
  `-a` includes stopped containers. The count of containers without a
  healthcheck is reported too, since an empty table and a table missing half
  the stack otherwise look the same.

### Fixed

- `dok inspect <name> --demo` answered as the api container whatever name was
  given, so `dok inspect redis --demo` showed api's id, image and healthcheck.
  The fixture now varies by service, and only api, postgres and grafana declare
  a healthcheck — grafana's is failing, which is what gives `dok health --demo`
  something to show.

## [0.1.5] - 2026-08-26

### Added

- `dok ports` — every published port in one flat table, sorted by host port, so
  "who owns 5432?" is a glance instead of a scan across wrapped lines. Skips
  exposed-but-unpublished ports, collapses the IPv4/IPv6 pair docker reports for
  a wildcard binding into one row, shows a `BIND` column only when some binding
  is narrower than every interface, and marks the case where two containers
  claim the same address, port and protocol.

### Changed

- The demo stack binds postgres to `127.0.0.1` rather than every interface,
  which is how a database is usually published and gives `dok ports --demo`
  something to show in its `BIND` column.

## [0.1.4] - 2026-08-23

### Added

- `dok uninstall` — removes dok the same way `dok update` replaces it. A
  standalone binary deletes itself (on Windows it is renamed aside and swept up
  a moment later, since Windows will not unlink a running image); a packaged
  one is left to its manager and that manager's removal command is printed.
  `--purge` also removes `~/.config/dok` and `~/.cache/dok`, `--dry-run` lists
  what would go, `-y` skips the prompt.

### Changed

- Install detection now recognises scoop, winget and pacman as well, and asks
  `dpkg` / `rpm` / `pacman` for the real package name instead of assuming
  `dok`.
- A config file that fails to parse no longer blocks `dok uninstall`: it warns
  and carries on with the defaults, so a bad config cannot trap anyone in an
  install they cannot leave. Every other command still refuses to guess.

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

[Unreleased]: https://github.com/alsaadii98/dok/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/alsaadii98/dok/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/alsaadii98/dok/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/alsaadii98/dok/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/alsaadii98/dok/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/alsaadii98/dok/releases/tag/v0.1.0
