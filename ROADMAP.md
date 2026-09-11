# Roadmap

What is done, what is next, and what is up for grabs. Items marked
**[good first issue]** are self-contained and need no deep knowledge of the
codebase.

## Shipped (0.1)

- [x] `ps` — compose grouping, state dots, health marks, ports, ages, ids
- [x] `images` — size and age gradients, dangling detection
- [x] `df` — per-category usage with used/reclaimable bars, top offenders
- [x] `inspect` — folded sections, masked secrets, healthcheck probe output
- [x] `logs` — merged streams, level colouring, JSON expansion, stderr marking
- [x] `top` — process tree by PPID
- [x] `tree` — projects, networks with IPs, volumes with mount points
- [x] `stats` — live TUI with bars and a total-CPU sparkline
- [x] `events` — colour-coded stream, `exec_*` hidden by default
- [x] Themes — 10 built-ins, palette + glyph set + layout, config overrides
- [x] `--demo` — canned example stack, no daemon required, drives the docs

## Shipped (0.2)

- [x] **`dok ports`** — one flat table of every published port across all
      containers, sorted by port. Answers "who owns 5432?" in one line.
      ([#8](https://github.com/alsaadii98/dok/issues/8))
- [x] **`dok history <image>`** — layer list with size bars and readable
      Dockerfile instructions. A CLI-shaped `dive` for spotting fat layers.
- [x] **`dok prune`** — what *would* be reclaimed, grouped by kind, before
      you commit to it. Never removes anything; there is no non-dry-run.
- [x] **`dok health`** — only containers with healthchecks: status, failing
      streak, last probe output.
- [x] **Shell completions** for bash/zsh/fish/nushell, generated in CI.
      ([#9](https://github.com/alsaadii98/dok/issues/9))

## Next up (0.3)

- [ ] **`--json` on every command** — same data, machine-readable, so `dok` can
      sit in scripts as well as in front of a human.
      ([#11](https://github.com/alsaadii98/dok/issues/11))
- [ ] **Remote contexts** — read `~/.docker/contexts` so `dok --context prod ps`
      works like the docker CLI. Reading containers on a remote host is where
      the wall-of-text problem is worst.

## Later

- [ ] `dok diff <container>` — files changed vs the image, coloured A/C/D
- [ ] `dok net <name>` — one network in detail: containers, aliases, gateway
- [ ] `dok compose` — project-level view: services, expected vs actual replicas,
      orphan containers
- [ ] `dok watch` — `ps` redrawn from the event stream instead of polling
- [ ] A man page generated from the clap definition **[good first issue]**
      ([#10](https://github.com/alsaadii98/dok/issues/10))
- [ ] Podman compatibility (the socket API is close enough to try)

## Themes wanted

More built-ins are welcome — see the theme section of
[CONTRIBUTING.md](CONTRIBUTING.md). Missing and frequently requested:
Everforest, Rosé Pine, Kanagawa, Ayu Light, Monokai, high-contrast/accessible.
**[good first issue]** — claim one in
[#12](https://github.com/alsaadii98/dok/issues/12).

## Explicit non-goals

- **Not a TUI you live in.** `dok stats` is one dashboard and stays that way;
  lazydocker and oxker already do that job well.
- **No container control.** No start/stop/exec/rm — `dok` reads, docker writes.
  This keeps it safe to run anywhere, including against production sockets.
- **No config for everything.** Themes are configurable; column sets and output
  formats stay opinionated, because that is the point of the tool.
