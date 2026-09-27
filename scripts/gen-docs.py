#!/usr/bin/env python3
"""Generate the dok manual (docs/man/*.html) from the binary itself.

    ./scripts/gen-docs.py            # uses target/release/dok, building if needed

Everything that can drift is read from dok rather than written down:

  * OPTIONS come from each command's `--help`
  * JSON OUTPUT is real `--demo --json` output
  * OUTPUT screenshots are the SVGs gen-screenshots.sh renders
  * the internals page lists the Docker API calls by grepping src/

The only hand-written content is each command's description and examples,
kept in COMMANDS below. Run this after gen-screenshots.sh; the release does
not run it, so commit the output.
"""

from __future__ import annotations

import datetime as dt
import html
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
MAN = DOCS / "man"
BIN = os.environ.get("DOK_BIN", str(ROOT / "target/release/dok"))
SITE = "https://dok.alsaadii98.com"

# Flags every subcommand inherits. They are documented once, on dok(1).
GLOBAL = {"--color", "--icons", "--theme", "--json", "--help", "--version", "--demo"}


# ── the authored part ───────────────────────────────────────────────────────
#
# desc      paragraphs, may contain `code`
# examples  (command, comment)
# shot      image under docs/img to use as OUTPUT, or None
# json      args for the JSON OUTPUT sample; None = the command has no JSON
#           form; "ndjson" in the tuple's first slot marks a stream
# also      related commands

COMMANDS: dict[str, dict] = {
    "ps": dict(
        group="Containers",
        desc=[
            "Lists containers grouped by compose project, showing the service name rather than the "
            "container name compose generates. Each row carries a short id, a state dot, a health "
            "mark when the container runs a healthcheck, published ports as `:host→container`, and "
            "a relative age.",
            "Columns shrink to fit the terminal instead of wrapping. Long names, images and port "
            "lists are capped at a share of the width and truncated past it, so one outlier cannot "
            "push the rest of the table off screen.",
        ],
        examples=[
            ("dok ps", "running containers, grouped by project"),
            ("dok ps -a", "include stopped containers"),
            ("dok ps --flat -s age", "one flat list, newest first"),
            ("dok ps -f postgres", "only rows whose name or image matches"),
        ],
        shot="ps.svg",
        json=("ps", "-a"),
        also=["ports", "health", "inspect", "tree"],
    ),
    "ports": dict(
        group="Containers",
        desc=[
            "One row per published binding, sorted by host port, so finding the owner of a port is "
            "a glance or a grep.",
            "Exposed-but-unpublished ports are skipped, since nothing on the host can reach them. A "
            "wildcard binding that Docker reports twice, as `0.0.0.0` and `::`, collapses into one "
            "row. The `BIND` column appears only when some binding is narrower than every "
            "interface. When two containers claim the same address, port and protocol, both rows "
            "are marked.",
        ],
        examples=[
            ("dok ports", "every published port"),
            ("dok ports -a", "include stopped containers"),
            ("dok ports | grep 5432", "who owns 5432"),
        ],
        shot="ports.svg",
        json=("ports",),
        also=["ps", "tree"],
    ),
    "health": dict(
        group="Containers",
        desc=[
            "Only the containers that declare a healthcheck, unhealthy first. For each one: the "
            "status, the failing streak, the probe interval, and the exit code and first line of "
            "the last probe's output. That last line is usually the reason a check fails, and "
            "`docker inspect` buries it.",
            "The summary counts containers without a healthcheck too, so an empty table and a "
            "table missing half the stack do not look the same. dok issues one inspect per "
            "container to read the probe log.",
        ],
        examples=[
            ("dok health", "every container with a healthcheck"),
            ("dok health -u", "only the failing ones"),
        ],
        shot="health.svg",
        json=("health",),
        also=["ps", "inspect", "events"],
    ),
    "history": dict(
        group="Images",
        desc=[
            "The layers of an image in build order, read top-down the way the Dockerfile was "
            "written, each with a bar showing its share of the image. Layers over 10% of the "
            "total are marked.",
            "Instructions are recovered from the classic builder's `/bin/sh -c #(nop)` wrapper and "
            "from BuildKit's direct form alike, and a multi-line `RUN` collapses onto one row. "
            "`-r` restores docker's newest-first order.",
        ],
        examples=[
            ("dok history api", "resolves a bare repo to any of its tags"),
            ("dok history api -r", "newest layer first"),
            ("dok history postgres:16-alpine --no-trunc", "keep long instructions whole"),
        ],
        shot="history.svg",
        json=("history", "api"),
        also=["images", "df"],
    ),
    "images": dict(
        group="Images",
        desc=[
            "Images sorted by size, with size and age gradients. There is one row per tag, so an "
            "image carrying two tags appears twice; the total counts unique image ids so it "
            "agrees with `docker system df`.",
            "Dangling images are marked reclaimable, and the last column counts the containers "
            "using each image.",
        ],
        examples=[
            ("dok images", "biggest first"),
            ("dok images -s age", "newest first"),
            ("dok images --dangling", "only untagged images"),
        ],
        shot="images.svg",
        json=("images",),
        also=["history", "df", "prune"],
    ),
    "df": dict(
        group="Disk",
        desc=[
            "Disk usage per category (images, containers, volumes and build cache) with a bar "
            "split into the part in use and the part that could be reclaimed.",
            "`-v` lists the biggest items in each category, which is usually where the answer to "
            "\u201cwhat ate the disk\u201d is.",
        ],
        examples=[
            ("dok df", "one bar per category"),
            ("dok df -v", "plus the biggest offenders"),
            ("dok df -v --top 20", "twenty per category"),
        ],
        shot="df.svg",
        json=("df", "-v"),
        also=["prune", "images"],
    ),
    "prune": dict(
        group="Disk",
        desc=[
            "Shows what `docker system prune` would remove, grouped by kind and sized, then prints "
            "the docker command that does it. dok never removes anything, and there is no flag "
            "that makes it.",
            "It draws the same line docker draws: dangling images by default, every unused image "
            "with `-a`, unused volumes only with `--volumes`. The built-in `bridge`, `host` and "
            "`none` networks are never listed, because docker refuses to remove them.",
        ],
        examples=[
            ("dok prune", "what docker system prune would take"),
            ("dok prune -a", "count every unused image"),
            ("dok prune -a --volumes", "and unused volumes"),
        ],
        shot="prune.svg",
        json=("prune",),
        also=["df", "images"],
    ),
    "inspect": dict(
        group="Containers",
        desc=[
            "The inspect JSON folded into identity, state, config, resources, network, mounts and "
            "labels. Privileged mode, added capabilities, OOM kills and failing healthchecks are "
            "called out in colour.",
            "Environment variables appear only with `--env`. Values whose keys look like "
            "credentials (`PASSWORD`, `SECRET`, `TOKEN`, `API_KEY` and similar) are replaced by "
            "their length unless `--show-secrets` is given. `--json` masks exactly the same way.",
        ],
        examples=[
            ("dok inspect api", "by name, service or id prefix"),
            ("dok inspect api --env", "include environment variables"),
            ("dok inspect api db", "several at once"),
        ],
        shot="inspect.svg",
        json=("inspect", "api"),
        also=["ps", "health", "top"],
    ),
    "logs": dict(
        group="Containers",
        desc=[
            "Interleaves the logs of several containers, each in a stable colour. The separator "
            "turns red for stderr, and JSON log lines are expanded into level, message and "
            "`key=value` pairs.",
            "With no container named, dok tails every running container.",
        ],
        examples=[
            ("dok logs", "every running container"),
            ("dok logs api db -f", "follow two of them"),
            ("dok logs api -n 200 -t", "200 lines with timestamps"),
            ("dok logs api -g error", "only matching lines"),
        ],
        shot="logs.svg",
        json=("ndjson", "logs", "-n", "4"),
        also=["events", "ps"],
    ),
    "top": dict(
        group="Containers",
        desc=[
            "The processes inside each container, nested under their parent PID so a process "
            "tree reads as one.",
            "`--ps-args` passes arguments to `ps` inside the container, and the column titles "
            "follow whatever it asks for. `--flat` drops the nesting.",
        ],
        examples=[
            ("dok top", "every running container"),
            ("dok top api", "one container"),
            ('dok top api --ps-args "-eo pid,ppid,rss,args"', "choose the columns"),
        ],
        shot="top.svg",
        json=("top", "api"),
        also=["inspect", "stats"],
    ),
    "tree": dict(
        group="System",
        desc=[
            "Three trees: compose projects and their containers, networks with the IP of each "
            "attached container, and volumes with where each is mounted.",
            "Network and volume membership is read from each container's own settings, since the "
            "list endpoints do not report it.",
        ],
        examples=[
            ("dok tree", "all three"),
            ("dok tree --only networks", "one section"),
            ("dok tree -a", "include stopped containers"),
        ],
        shot="tree.svg",
        json=("tree",),
        also=["ps", "ports"],
    ),
    "events": dict(
        group="System",
        desc=[
            "The daemon event stream, colour-coded by object type and action, with exit codes, "
            "signals and health changes picked out.",
            "`exec_*` events fire on every `docker exec` and every healthcheck probe, and would "
            "drown the lifecycle events people watch for, so they are hidden unless `--exec` is "
            "given. `--since` and `--until` take timestamps or durations such as `2h`.",
        ],
        examples=[
            ("dok events", "stream from now"),
            ("dok events --since 2h", "replay the last two hours"),
            ("dok events -T container,volume", "only these object types"),
        ],
        shot="events.svg",
        json=("ndjson", "events"),
        also=["logs", "health"],
    ),
    "stats": dict(
        group="System",
        desc=[
            "A live CPU, memory and IO dashboard. It is the one full-screen command: `q` quits "
            "and `s` cycles the sort.",
            "It has no JSON form. With `--json` it exits with an error rather than drawing a "
            "dashboard into a pipe; use `dok ps --json` for a snapshot.",
        ],
        examples=[
            ("dok stats", "every running container"),
            ("dok stats api db --interval 500", "two containers, twice a second"),
        ],
        shot=None,
        json=None,
        also=["top", "ps"],
    ),
    "themes": dict(
        group="Setup",
        desc=[
            "Lists the built-in themes and any defined in the config file, each with a row of "
            "swatches. A theme carries a palette of nine colour roles, a glyph set and a layout, "
            "so the `ascii` theme stays pure ASCII for CI logs and serial consoles.",
            "`--preview` renders a sample table in every theme. `--init` writes a starter config "
            "to `~/.config/dok/config.toml`.",
        ],
        examples=[
            ("dok themes", "list with swatches"),
            ("dok themes --preview", "a sample table in each"),
            ("dok ps --theme gruvbox", "use one for a single command"),
        ],
        shot="themes.svg",
        json=("themes",),
        also=["dok"],
    ),
    "completions": dict(
        group="Setup",
        desc=[
            "Prints a completion script for bash, zsh, fish, nushell, powershell or elvish. It is "
            "generated from the same definition the CLI is built from, so it cannot fall behind "
            "the commands and flags that exist.",
            "Homebrew, the `.deb` and the `.rpm` install completions already. Every release also "
            "attaches a tarball with all six.",
        ],
        examples=[
            ('dok completions zsh > "${fpath[1]}/_dok"', "zsh"),
            ("dok completions bash > /etc/bash_completion.d/dok", "bash"),
            ("dok completions fish > ~/.config/fish/completions/dok.fish", "fish"),
        ],
        shot=None,
        json=None,
        also=["dok"],
    ),
    "update": dict(
        group="Setup",
        desc=[
            "Checks GitHub for a newer release. A standalone binary is replaced in place, after "
            "its archive is verified against the release's `.sha256`. A packaged install is left "
            "alone and the package manager's own upgrade command is printed instead.",
            "dok also checks at most once a day and prints one dim line after a command's output. "
            "It never does so when output is piped or `--json` is set. `DOK_NO_UPDATE_CHECK=1` "
            "turns it off.",
        ],
        examples=[
            ("dok update --check", "report, install nothing"),
            ("dok update", "install, after asking"),
            ("dok update -y", "install without asking"),
        ],
        shot=None,
        json=None,
        also=["uninstall"],
    ),
    "uninstall": dict(
        group="Setup",
        desc=[
            "Removes dok the way it was installed. A standalone binary deletes itself. A packaged "
            "one is left to its manager, and that manager's removal command is printed.",
            "`--purge` also removes `~/.config/dok` and `~/.cache/dok`. `--dry-run` lists what "
            "would go and removes nothing.",
        ],
        examples=[
            ("dok uninstall --dry-run", "list, remove nothing"),
            ("dok uninstall --purge", "and the config and cache"),
        ],
        shot=None,
        json=None,
        also=["update"],
    ),
}

# What each page should be found for. The title answers the question the way
# it is typed into a search box; the description is one plain sentence; the
# keywords are the phrasings people actually use. No stuffing: each list is a
# handful of real variants of the same question.
SEO: dict[str, tuple[str, str, list[str]]] = {
    "ps": ("A readable docker ps, grouped by compose project",
           "dok ps lists Docker containers grouped by compose project, with service names, health, ports and ages that fit the terminal instead of wrapping.",
           ["docker ps alternative", "readable docker ps", "docker ps grouped by compose project", "pretty docker ps", "docker ps format"]),
    "ports": ("Find which Docker container is using a port",
              "dok ports prints every published port across all containers in one table sorted by host port, so finding who owns 5432 takes one glance.",
              ["docker which container is using port", "docker port already in use", "find docker container by port", "docker list published ports", "docker port 5432 in use"]),
    "health": ("See why a Docker healthcheck is failing",
               "dok health shows only containers with a healthcheck, unhealthy first, with the failing streak and the output of the last probe.",
               ["docker healthcheck failing", "docker container unhealthy why", "docker healthcheck log", "docker health status", "debug docker healthcheck"]),
    "history": ("Find which layer is making a Docker image large",
                "dok history lists image layers in build order with a size bar each and the real Dockerfile instruction, marking layers over 10% of the image.",
                ["docker image too big", "docker history readable", "which layer makes docker image large", "docker image layer size", "dive alternative"]),
    "images": ("Docker images sorted by size, dangling marked",
               "dok images lists Docker images sorted by size with age, marks dangling images as reclaimable and counts the containers using each.",
               ["docker images sorted by size", "docker dangling images", "docker list images by size", "largest docker images"]),
    "df": ("What is taking up Docker disk space",
           "dok df breaks Docker disk usage down by images, containers, volumes and build cache, and lists the biggest items in each.",
           ["docker disk space", "what is using docker disk space", "docker system df explained", "docker disk usage by image", "docker build cache size"]),
    "prune": ("See what docker system prune will delete, before it does",
              "dok prune previews exactly what docker system prune would remove, grouped and sized, without removing anything.",
              ["docker system prune dry run", "what does docker system prune delete", "docker prune preview", "docker system prune -a what is removed", "docker cleanup safely"]),
    "inspect": ("A readable docker inspect, with secrets masked",
                "dok inspect folds the docker inspect JSON into readable sections and masks credential-looking environment values by default.",
                ["docker inspect readable", "docker inspect format", "docker inspect environment variables", "docker inspect healthcheck", "docker inspect mounts"]),
    "logs": ("Tail logs from several Docker containers at once",
             "dok logs interleaves the logs of several containers in stable colours, marks stderr, and expands JSON log lines into readable fields.",
             ["docker logs multiple containers", "docker logs all containers", "docker compose logs colour", "docker logs json pretty print", "tail docker logs"]),
    "top": ("See the processes inside a Docker container as a tree",
            "dok top shows the processes running inside Docker containers, nested under their parent PID.",
            ["docker top", "processes in docker container", "docker container process tree", "docker exec ps alternative"]),
    "tree": ("Docker networks, volumes and compose projects as a tree",
             "dok tree shows compose projects, networks with each container's IP, and volumes with where they are mounted.",
             ["docker network containers ip", "which containers use a docker volume", "docker network list containers", "docker compose project overview"]),
    "events": ("Watch the Docker event stream, colour-coded",
               "dok events streams Docker daemon events colour-coded by type and action, with exec and healthcheck noise hidden by default.",
               ["docker events", "docker events filter", "watch docker events", "docker events since"]),
    "stats": ("A live Docker CPU and memory dashboard",
              "dok stats is a live dashboard of CPU, memory and IO per Docker container.",
              ["docker stats", "docker container cpu memory usage", "docker stats dashboard"]),
    "themes": ("Colour themes for Docker output in the terminal",
               "dok themes lists ten built-in themes that change the palette, glyphs and table layout of dok's Docker output.",
               ["docker cli colors", "docker output colour", "terminal themes docker"]),
    "completions": ("Shell completions for bash, zsh, fish and nushell",
                    "dok completions prints a completion script for bash, zsh, fish, nushell, powershell or elvish.",
                    ["dok zsh completion", "dok bash completion", "dok fish completion"]),
    "update": ("Update dok to the latest release",
               "dok update checks GitHub for a newer release and replaces the binary after verifying its checksum, or prints your package manager's command.",
               ["update dok", "dok upgrade"]),
    "uninstall": ("Uninstall dok cleanly",
                  "dok uninstall removes dok the way it was installed, or prints your package manager's removal command.",
                  ["uninstall dok", "remove dok"]),
}

ORDER = [
    ("Containers", ["ps", "ports", "health", "inspect", "logs", "top"]),
    ("Images", ["images", "history"]),
    ("Disk", ["df", "prune"]),
    ("System", ["tree", "events", "stats"]),
    ("Setup", ["themes", "completions", "update", "uninstall"]),
]


# ── reading the binary ──────────────────────────────────────────────────────

def run(*args: str) -> str:
    out = subprocess.run([BIN, *args], capture_output=True, text=True, env={**os.environ, "NO_COLOR": "1"})
    return out.stdout


def version() -> str:
    return run("--version").strip().split()[-1]


def parse_help(text: str) -> dict:
    """Split clap's long help into about, usage, arguments and options."""
    lines = text.splitlines()
    about = lines[0].strip() if lines else ""
    usage = next((l.split("Usage:", 1)[1].strip() for l in lines if l.startswith("Usage:")), "")
    sections: dict[str, list[dict]] = {"Arguments": [], "Options": []}
    cur = None
    entry = None
    for line in lines:
        head = line.rstrip(":")
        if head in sections and line.endswith(":"):
            cur, entry = head, None
            continue
        if cur is None:
            continue
        if not line.strip():
            if entry is not None:
                entry["body"].append("")
            continue
        indent = len(line) - len(line.lstrip())
        if indent < 8:
            entry = {"flag": line.strip(), "body": []}
            sections[cur].append(entry)
        elif entry is not None:
            entry["body"].append(line.strip())
    for sec in sections.values():
        for e in sec:
            while e["body"] and not e["body"][-1]:
                e["body"].pop()
    return {"about": about, "usage": usage, **sections}


def long_flag(flag: str) -> str:
    m = re.search(r"--[a-z-]+", flag)
    return m.group(0) if m else flag


def json_sample(spec: tuple) -> tuple[str, str, str] | None:
    """(command line, highlighted html, note) for a command's JSON output."""
    if spec is None:
        return None
    stream = spec[0] == "ndjson"
    args = list(spec[1:] if stream else spec)
    raw = run(*args, "--demo", "--json")
    shown = "dok " + " ".join(args) + " --json"
    if stream:
        rows = [l for l in raw.splitlines() if l.strip()][:2]
        body = "\n".join(highlight(json.dumps(json.loads(r), ensure_ascii=False)) for r in rows)
        return shown, body, "NDJSON: one object per line, flushed as it arrives. First two lines shown."
    data = json.loads(raw)
    trimmed, note = trim(data)
    body = highlight(json.dumps(trimmed, indent=2, ensure_ascii=False))
    return shown, body, note


def trim(v, depth=0):
    """Keep samples readable: long arrays show their first two elements."""
    notes = []

    def walk(x, d):
        if isinstance(x, list):
            if len(x) > 2 and d < 3:
                notes.append(len(x))
                return [walk(i, d + 1) for i in x[:2]]
            return [walk(i, d + 1) for i in x]
        if isinstance(x, dict):
            return {k: walk(val, d + 1) for k, val in x.items()}
        return x

    out = walk(v, depth)
    note = "Longer arrays are cut to their first two elements here." if notes else ""
    return out, note


def highlight(src: str) -> str:
    """Colour JSON with the roles dok itself uses for the same kinds of value."""
    out = []
    token = re.compile(r'("(?:\\.|[^"\\])*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b')
    pos = 0
    for m in token.finditer(src):
        out.append(html.escape(src[pos:m.start()]))
        s, colon, num, lit = m.groups()
        if s is not None:
            cls = "k" if colon else "s"
            out.append(f'<span class="{cls}">{html.escape(s)}</span>{html.escape(colon or "")}')
        elif num is not None:
            out.append(f'<span class="n">{num}</span>')
        else:
            out.append(f'<span class="b">{lit}</span>')
        pos = m.end()
    out.append(html.escape(src[pos:]))
    return "".join(out)


def api_calls() -> list[str]:
    """Every Docker API call dok makes, straight from the source."""
    calls = set()
    for f in (ROOT / "src").rglob("*.rs"):
        calls.update(re.findall(r"docker\.([a-z_]+)\(", f.read_text()))
    calls.discard("clone")
    return sorted(calls)


def src_excerpt(path: str, start: str, end: str = "\n}\n") -> str:
    text = (ROOT / path).read_text()
    i = text.index(start)
    j = text.index(end, i) + len(end)
    return text[i:j].rstrip()


# ── rendering ───────────────────────────────────────────────────────────────

def md(s: str) -> str:
    """The small subset of inline markup the descriptions use: `code`."""
    s = html.escape(s, quote=False)
    return re.sub(r"`([^`]+)`", r'<code translate="no">\1</code>', s)


GH_SVG = (
    '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 .3a12 12 0 0 0-3.8 23.4c.6.1.8-.3.8-.6v-2.1c-3.3.7-4-1.6-4-1.6-.6-1.4-1.4-1.8-1.4-1.8-1-.7.1-.7.1-.7 1.2.1 1.8 1.2 1.8 1.2 1 1.8 2.8 1.3 3.5 1 .1-.8.4-1.3.7-1.6-2.7-.3-5.5-1.3-5.5-5.9 0-1.3.5-2.4 1.2-3.2-.1-.3-.5-1.5.1-3.2 0 0 1-.3 3.3 1.2a11.5 11.5 0 0 1 6 0C17.3 4.7 18.3 5 18.3 5c.6 1.7.2 2.9.1 3.2.8.8 1.2 1.9 1.2 3.2 0 4.6-2.8 5.6-5.5 5.9.4.4.8 1.1.8 2.2v3.3c0 .3.2.7.8.6A12 12 0 0 0 12 .3"/></svg>'
)


def nav(current: str) -> str:
    def a(href, label, key, cls=""):
        cur = ' aria-current="page"' if key == current else ""
        c = f' class="{cls}"' if cls else ""
        return f'<li{c}><a href="{href}"{cur}>{label}</a></li>'

    return f"""<header class="nav">
  <div class="wrap">
    <a class="brand" href="/">dok</a>
    <ul>
      {a("/#commands", "commands", "commands", "opt")}
      {a("/#json", "json", "json", "opt")}
      {a("/#install", "install", "install", "opt2")}
      {a("/man/", "manual", "man")}
      <li><a class="gh" href="https://github.com/alsaadii98/dok" aria-label="dok on GitHub">{GH_SVG}<span class="opt2">GitHub</span></a></li>
    </ul>
  </div>
</header>"""


FOOT = """<footer class="foot">
  <div class="wrap">
    <a class="brand" href="/">dok</a>
    <a href="/man/">Manual</a>
    <a href="https://github.com/alsaadii98/dok">GitHub</a>
    <a href="https://crates.io/crates/dok-cli">crates.io</a>
    <a href="https://github.com/alsaadii98/dok/releases">Releases</a>
    <a href="https://github.com/alsaadii98/dok/blob/main/LICENSE">MIT</a>
    <span>&copy; 2026 <a href="https://github.com/alsaadii98">alsaadii98</a></span>
  </div>
</footer>"""


def head(title: str, desc: str, path: str, keywords: list[str] | None = None, crumb: str | None = None) -> str:
    kw = f'<meta name="keywords" content="{html.escape(", ".join(keywords))}">\n' if keywords else ""
    trail = [("dok", "/"), ("Manual", "/man/")]
    if crumb:
        trail.append((crumb, path))
    ld = [
        {
            "@context": "https://schema.org",
            "@type": "BreadcrumbList",
            "itemListElement": [
                {"@type": "ListItem", "position": i + 1, "name": n, "item": f"{SITE}{u}"}
                for i, (n, u) in enumerate(trail)
            ],
        },
        {
            "@context": "https://schema.org",
            "@type": "TechArticle",
            "headline": title,
            "description": desc,
            "url": f"{SITE}{path}",
            "inLanguage": "en",
            "about": {"@type": "SoftwareApplication", "name": "dok", "url": f"{SITE}/",
                      "applicationCategory": "DeveloperApplication", "operatingSystem": "macOS, Linux, Windows"},
            "author": {"@type": "Person", "name": "alsaadii98", "url": "https://github.com/alsaadii98"},
        },
    ]
    ld_html = "".join(f'<script type="application/ld+json">{json.dumps(x)}</script>\n' for x in ld)
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{html.escape(desc)}">
{kw}<meta name="robots" content="index, follow, max-image-preview:large">
<link rel="canonical" href="{SITE}{path}">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<meta name="theme-color" content="#070708">
<meta name="color-scheme" content="dark">
<meta property="og:type" content="article">
<meta property="og:site_name" content="dok">
<meta property="og:title" content="{html.escape(title)}">
<meta property="og:description" content="{html.escape(desc)}">
<meta property="og:url" content="{SITE}{path}">
<meta property="og:image" content="{SITE}/img/dok-social.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta name="twitter:card" content="summary_large_image">
<link rel="preload" href="/fonts/GeistMono-Variable.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="/fonts/Geist-Variable.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="/assets/site.css">
<link rel="stylesheet" href="/assets/man.css">
<script src="/assets/site.js" defer></script>
{ld_html}</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<div class="atmos" aria-hidden="true"></div>
<p id="live" class="sr" aria-live="polite"></p>
"""


def sidebar(current: str) -> str:
    groups = []
    for group, keys in ORDER:
        items = "".join(
            f'<li><a href="/man/{k}/"{" aria-current=\"page\"" if k == current else ""}>{k}</a></li>'
            for k in keys
        )
        groups.append(f'<p class="grp">{group}</p><ul>{items}</ul>')
    top = [
        ("dok", "/man/", "dok(1) overview"),
        ("internals", "/man/internals/", "dok(7) internals"),
    ]
    tops = "".join(
        f'<li><a href="{h}"{" aria-current=\"page\"" if k == current else ""}>{label}</a></li>'
        for k, h, label in top
    )
    return f"""<nav class="side" aria-label="Manual">
  <ul class="top">{tops}</ul>
  {"".join(groups)}
</nav>"""


def page(key: str, title: str, desc: str, path: str, section: str, body: str, ver: str, today: str,
         keywords: list[str] | None = None, crumb: str | None = None) -> str:
    ref = section
    return f"""{head(title, desc, path, keywords, crumb)}{nav("man")}
<div class="wrap man">
  {sidebar(key)}
  <main id="main" class="article">
    <div class="manhead" aria-hidden="true"><span>{ref}</span><span>dok manual</span><span>{ref}</span></div>
{body}
    <div class="manfoot"><span>dok {ver}</span><span>{today}</span><span>{ref}</span></div>
  </main>
</div>
{FOOT}
</body>
</html>
"""


def options_html(entries: list[dict]) -> str:
    rows = []
    for e in entries:
        body = [l for l in e["body"]]
        text, meta = [], []
        for l in body:
            if l.startswith("[default:") or l.startswith("[possible values:"):
                meta.append(l.strip("[]"))
            elif l.startswith("Possible values:"):
                meta.append("possible values:")
            elif l.startswith("- "):
                meta.append(l[2:])
            elif l:
                text.append(l)
        m = "".join(f'<span class="meta">{md(x)}</span>' for x in meta)
        rows.append(f'<dt><code translate="no">{html.escape(e["flag"])}</code></dt><dd>{md(" ".join(text))}{m}</dd>')
    return f'<dl class="opts">{"".join(rows)}</dl>'


def cmd_page(key: str, spec: dict, ver: str, today: str) -> str:
    h = parse_help(run(key, "--help"))
    own = [e for e in h["Options"] if long_flag(e["flag"]) not in GLOBAL]
    ref = f"DOK-{key.upper()}(1)"

    parts = [f'<h1><span class="p">$</span> dok {key}</h1>', f'<p class="about">{md(h["about"])}</p>']

    parts.append('<h2 id="synopsis">Synopsis</h2>')
    parts.append(f'<div class="code"><pre><code translate="no">dok {html.escape(h["usage"].split("dok ", 1)[-1])}</code></pre></div>')

    parts.append('<h2 id="description">Description</h2>')
    parts.append('<div class="prose">' + "".join(f"<p>{md(p)}</p>" for p in spec["desc"]) + "</div>")

    if h["Arguments"]:
        parts.append('<h2 id="arguments">Arguments</h2>')
        parts.append(options_html(h["Arguments"]))

    parts.append('<h2 id="options">Options</h2>')
    parts.append(options_html(own) if own else '<p class="prose">None beyond the global options.</p>')
    parts.append('<p class="prose note">Every command also takes the <a href="/man/#options">global options</a>: <code>--json</code>, <code>--color</code>, <code>--icons</code> and <code>--theme</code>.</p>')

    parts.append('<h2 id="examples">Examples</h2>')
    ex = []
    for cmd, note in spec["examples"]:
        ex.append(
            f'<div class="code"><button class="copy" type="button" data-copy="{html.escape(cmd)}" '
            f'aria-label="Copy command">copy</button><pre><code translate="no"><span class="p">$ </span>{html.escape(cmd)}'
            f'  <span class="c"># {html.escape(note)}</span></code></pre></div>'
        )
    parts.append('<div class="stack">' + "".join(ex) + "</div>")

    if spec["shot"]:
        f = DOCS / "img" / spec["shot"]
        w, hgt = svg_size(f)
        parts.append('<h2 id="output">Output</h2>')
        parts.append(
            f'<figure class="shot"><img src="/img/{spec["shot"]}" width="{w}" height="{hgt}" loading="lazy" '
            f'decoding="async" alt="Output of dok {key} against the demo stack"><figcaption>Rendered from '
            f'<code>dok {key} --demo</code>, dok\u2019s built-in example stack.</figcaption></figure>'
        )

    js = json_sample(spec["json"])
    if js:
        cmdline, body, note = js
        parts.append('<h2 id="json">JSON output</h2>')
        parts.append(
            f'<div class="code"><button class="copy" type="button" data-copy="{html.escape(cmdline)}" aria-label="Copy command">copy</button>'
            f'<pre><code translate="no"><span class="p">$ </span>{html.escape(cmdline)}\n{body}</code></pre></div>'
        )
        if note:
            parts.append(f'<p class="prose note">{md(note)}</p>')
    elif key == "stats":
        parts.append('<h2 id="json">JSON output</h2>')
        parts.append('<p class="prose">None. See the description above.</p>')

    parts.append('<h2 id="see-also">See also</h2>')
    links = ", ".join(
        f'<a href="/man/{"" if k == "dok" else k + "/"}">dok{"" if k == "dok" else "-" + k}(1)</a>' for k in spec["also"]
    )
    parts.append(f'<p class="prose">{links}, <a href="/man/internals/">dok(7)</a></p>')

    body = "\n".join("    " + p for p in parts)
    t, d, kw = SEO[key]
    return page(key, f"{t} | dok {key}", d, f"/man/{key}/", ref, body, ver, today, kw, f"dok {key}")


def svg_size(f: Path) -> tuple[int, int]:
    m = re.search(r'viewBox="0 0 ([\d.]+) ([\d.]+)"', f.read_text()[:600])
    return (round(float(m.group(1))), round(float(m.group(2)))) if m else (820, 400)


def overview_page(ver: str, today: str) -> str:
    h = parse_help(run("--help"))
    glob = [e for e in h["Options"] if long_flag(e["flag"]) not in {"--help", "--version"}]
    index = []
    for group, keys in ORDER:
        rows = "".join(
            f'<a class="row" href="/man/{k}/"><code translate="no">dok {k}</code><span>{md(parse_help(run(k, "--help"))["about"])}</span></a>'
            for k in keys
        )
        index.append(f'<h3>{group}</h3><div class="idx">{rows}</div>')

    body = f"""    <h1><span class="p">$</span> dok</h1>
    <p class="about">Docker output, made readable.</p>
    <h2 id="synopsis">Synopsis</h2>
    <div class="code"><pre><code translate="no">dok [OPTIONS] &lt;COMMAND&gt;</code></pre></div>
    <h2 id="description">Description</h2>
    <div class="prose">
      <p>dok reads the Docker daemon&#8217;s API directly and renders containers, images, logs, disk usage and events for a person to read. Every command prints one static screen and exits, except <a href="/man/stats/"><code>stats</code></a>, so output pipes into <code>grep</code>, <code>less</code> and <code>jq</code> like any other Unix tool.</p>
      <p>dok is read-only. It never starts, stops, execs into or removes anything, which makes it safe to point at a production socket. <a href="/man/internals/">dok(7)</a> lists every Docker API call it makes.</p>
    </div>
    <h2 id="commands">Commands</h2>
    {"".join(index)}
    <h2 id="options">Global options</h2>
    {options_html(glob)}
    <p class="prose note">Colour and icons turn off by themselves when output is not a terminal, and <code>NO_COLOR</code> is honoured.</p>
    <h2 id="json">JSON output</h2>
    <div class="prose">
      <p><code>--json</code> works on every command that has something to show. It carries dok&#8217;s data rather than the raw API: the compose project, the service name, the health verdict, the reclaimable flag. Run <code>docker inspect</code> for the raw payload.</p>
      <p>Sizes and ages arrive as both the number and the rendering, so nothing has to be parsed back:</p>
    </div>
    <div class="code"><pre><code translate="no">{highlight(json.dumps({"size": {"bytes": 1181116006, "human": "1.1GB"}}, indent=2))}</code></pre></div>
    <div class="prose">
      <p>An absent value is <code>null</code>, never an empty string. <a href="/man/logs/"><code>logs</code></a> and <a href="/man/events/"><code>events</code></a> are streams and emit NDJSON, one object per line. <code>--json</code> implies <code>--color never</code> and suppresses the update notice, since anything after the document would break a parser.</p>
    </div>
    <h2 id="config">Configuration</h2>
    <div class="prose"><p><code>dok themes --init</code> writes <code>~/.config/dok/config.toml</code>:</p></div>
    <div class="code"><pre><code translate="no"><span class="k">theme</span> = <span class="s">"mine"</span>
<span class="k">icons</span> = <span class="s">"nerd"</span>          <span class="c"># auto | nerd | unicode | none</span>

[themes.mine]
<span class="k">base</span>   = <span class="s">"gruvbox"</span>      <span class="c"># start from any built-in</span>
<span class="k">glyphs</span> = <span class="s">"heavy"</span>        <span class="c"># unicode | ascii | heavy | slim</span>
<span class="k">layout</span> = <span class="s">"grid"</span>         <span class="c"># default | ruled | grid | quiet | ascii</span>
<span class="k">green</span>  = <span class="s">"#00ff00"</span>      <span class="c"># override any palette role</span></code></pre></div>
    <div class="prose"><p>Theme precedence is <code>--theme</code>, then <code>DOK_THEME</code>, then the config file, then <code>default</code>.</p></div>
    <h2 id="environment">Environment</h2>
    <dl class="opts">
      <dt><code translate="no">DOCKER_HOST</code></dt><dd>Where the daemon is. Unset means <code>/var/run/docker.sock</code>, or the named pipe on Windows.</dd>
      <dt><code translate="no">DOK_THEME</code></dt><dd>Theme name, below <code>--theme</code> and above the config file.</dd>
      <dt><code translate="no">NO_COLOR</code></dt><dd>Any value turns colour off, as <code>--color never</code> does.</dd>
      <dt><code translate="no">DOK_NERD_FONT</code></dt><dd><code>1</code> forces Nerd Font glyphs, <code>0</code> refuses them.</dd>
      <dt><code translate="no">DOK_NO_UPDATE_CHECK</code></dt><dd>Any value stops the once-a-day release check.</dd>
      <dt><code translate="no">DOK_DEMO</code></dt><dd>Same as <code>--demo</code>: render the built-in example stack, no daemon needed.</dd>
    </dl>
    <h2 id="see-also">See also</h2>
    <p class="prose"><a href="/man/internals/">dok(7)</a>, <a href="https://github.com/alsaadii98/dok">github.com/alsaadii98/dok</a></p>"""
    return page("dok", "dok manual: every command, option and output format",
                "The dok manual. Every command, its options, real output and JSON shape, generated from the binary.",
                "/man/", "DOK(1)", body, ver, today,
                ["dok manual", "docker cli readable output", "docker ps json output", "docker cli tool", "dok documentation"])


def internals_page(ver: str, today: str) -> str:
    calls = api_calls()
    call_rows = "".join(f"<li><code translate=\"no\">{c}</code></li>" for c in calls)
    connect = src_excerpt("src/dk.rs", "pub fn connect()")
    width = src_excerpt("src/fmt.rs", "pub fn visible_width")
    palette = src_excerpt("src/theme.rs", "/// Nine semantic roles.", "\n}\n")

    def block(code: str) -> str:
        return f'<div class="code"><pre><code translate="no">{html.escape(code)}</code></pre></div>'

    stages = [
        ("socket", "/var/run/docker.sock", "or DOCKER_HOST, or a Windows named pipe"),
        ("client", "bollard", "the Docker API over HTTP, in Rust"),
        ("views", "src/dk.rs", "names, compose labels, state, health"),
        ("layout", "src/table.rs", "measure, cap, shrink, pad"),
        ("paint", "src/theme.rs", "roles, glyphs, layout"),
        ("out", "your terminal", "or a pipe, or --json"),
    ]
    flow = "".join(
        f'<li><span class="st">{s}</span><code translate="no">{html.escape(c)}</code><span class="dsc">{html.escape(d)}</span></li>'
        for s, c, d in stages
    )

    body = f"""    <h1><span class="p">$</span> man 7 dok</h1>
    <p class="about">How dok turns the Docker API into something a person can read.</p>
    <h2 id="pipeline">Pipeline</h2>
    <ol class="flow">{flow}</ol>
    <h2 id="socket">No shell-out</h2>
    <div class="prose">
      <p>dok never runs the <code>docker</code> CLI. It talks to the daemon&#8217;s HTTP API through <a href="https://github.com/fussybeaver/bollard">bollard</a>, over the same socket the CLI uses, so it needs nothing installed beyond a reachable daemon.</p>
      <p><code>--demo</code> builds a client pointed at a dead port and answers every query from fixtures instead, which is why it works on a machine with no Docker at all:</p>
    </div>
    {block(connect)}
    <h2 id="read-only">Read-only, by construction</h2>
    <div class="prose">
      <p>These are all the Docker API calls in dok&#8217;s source. The list is generated from the code every time this page is built:</p>
    </div>
    <ul class="calls">{call_rows}</ul>
    <div class="prose"><p>Every one of them reads. There is no create, start, stop, kill, remove or prune anywhere in the codebase, so a bug in dok cannot change a container.</p></div>
    <h2 id="layout">Tables that fit</h2>
    <div class="prose">
      <p>Every cell arrives already coloured, so its length in bytes says nothing about its width on screen. All width maths goes through one function that skips escape sequences:</p>
    </div>
    {block(width)}
    <div class="prose">
      <p>With widths measured, <code>table.rs</code> caps each column at a share of the terminal, shrinks flexible columns widest-first until the table fits, and pads. Truncation copies escape sequences through whole, so a cut never leaves a stray <code>ESC</code> byte behind.</p>
    </div>
    <h2 id="themes">Themes are roles, not colours</h2>
    <div class="prose">
      <p>No command names a colour. They ask for a role, and the theme decides what it looks like. Structural glyphs (tree stubs, bars, arrows, state dots) come from the theme too, which is how the <code>ascii</code> theme stays pure ASCII.</p>
    </div>
    {block(palette)}
    <h2 id="output">Output modes</h2>
    <div class="prose">
      <p>When stdout is a terminal, dok colours output and picks an icon set. Piped, both turn off. <code>--json</code> replaces the table with a document, or NDJSON for the two streaming commands. <code>dok stats</code> alone takes over the screen, and refuses <code>--json</code> rather than drawing into a pipe.</p>
    </div>
    <h2 id="see-also">See also</h2>
    <p class="prose"><a href="/man/">dok(1)</a>, <a href="https://github.com/alsaadii98/dok/blob/main/CONTRIBUTING.md">CONTRIBUTING.md</a></p>"""
    return page("internals", "How dok works: the Docker socket, bollard, and ANSI-aware tables",
                "How dok reads the Docker API without shelling out, stays read-only, and lays out coloured tables that fit the terminal.",
                "/man/internals/", "DOK(7)", body, ver, today,
                ["docker api rust", "bollard docker", "ansi aware table rust", "read-only docker tool"], "internals")


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    print(f"wrote {path.relative_to(ROOT)}")


def sitemap(today: str) -> None:
    urls = ["/", "/man/", "/man/internals/"] + [f"/man/{k}/" for _, ks in ORDER for k in ks]
    rows = "\n".join(
        f"  <url><loc>{SITE}{u}</loc><lastmod>{today}</lastmod></url>" for u in urls
    )
    write(DOCS / "sitemap.xml",
          f'<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n{rows}\n</urlset>\n')


def main() -> None:
    if not Path(BIN).exists():
        subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    ver = version()
    today = dt.date.today().isoformat()
    missing = [k for _, ks in ORDER for k in ks if k not in COMMANDS or k not in SEO]
    if missing:
        sys.exit(f"ORDER names commands with no entry: {missing}")
    listed = {k for _, ks in ORDER for k in ks}
    real = set(re.findall(r"^  ([a-z]+)\s", run("--help").split("Commands:")[1].split("Options:")[0], re.M)) - {"help"}
    if real != listed:
        sys.exit(f"manual and binary disagree. binary only: {sorted(real - listed)}, manual only: {sorted(listed - real)}")

    for _, keys in ORDER:
        for k in keys:
            write(MAN / k / "index.html", cmd_page(k, COMMANDS[k], ver, today))
    write(MAN / "index.html", overview_page(ver, today))
    write(MAN / "internals" / "index.html", internals_page(ver, today))
    write(DOCS / "index.html", home_page(ver, today))
    sitemap(today)



# ── landing page ────────────────────────────────────────────────────────────
#
# Written here rather than by hand so the numbers on it (API calls, commands,
# themes) are counted from the binary, and so the visible FAQ is built from
# the same list as its structured-data twin.

BROWSE = ["ps", "ports", "health", "history", "prune", "images", "df", "inspect", "logs", "top", "tree", "events", "themes"]
ALIAS = {"ps": "ls", "images": "img", "df": "du"}

INSTALL = [
    ("brew", "homebrew", "brew install alsaadii98/tap/dok", "macOS and Linux. Installs shell completions too."),
    ("cargo", "cargo", "cargo install dok-cli", "Any platform with Rust 1.88+. The crate is dok-cli; the binary is dok."),
    ("arch", "arch", "git clone https://github.com/alsaadii98/dok\ncd dok/packaging/aur\nmakepkg -si -p PKGBUILD-bin", "Prebuilt binary. Drop -p PKGBUILD-bin to build from source."),
    ("deb", "debian", "curl -LO https://github.com/alsaadii98/dok/releases/latest/download/dok_amd64.deb\nsudo dpkg -i dok_amd64.deb", "Debian and Ubuntu. Includes completions for bash, zsh and fish."),
    ("rpm", "fedora", "curl -LO https://github.com/alsaadii98/dok/releases/latest/download/dok.x86_64.rpm\nsudo rpm -i dok.x86_64.rpm", "Fedora and RHEL. Includes completions."),
    ("alpine", "alpine", "wget https://github.com/alsaadii98/dok/releases/latest/download/dok-x86_64.apk\napk add --allow-untrusted ./dok-x86_64.apk", "Static musl binary, no dependencies. aarch64 too."),
    ("nix", "nix", "nix run github:alsaadii98/dok", "Or nix profile install github:alsaadii98/dok."),
    ("scoop", "windows", "scoop bucket add dok https://github.com/alsaadii98/dok\nscoop install dok/dok", "This repository doubles as a scoop bucket."),
    ("src", "source", "git clone https://github.com/alsaadii98/dok\ncd dok && cargo build --release", "The binary lands in target/release/dok."),
]

COMPARE = [
    ("Static, pipeable output", ["yes", "yes", "yes", "no, full-screen TUI"]),
    ("Grouped by compose project", ["yes", "no", "no", "partial"]),
    ("Readable inspect, secrets masked", ["yes", "no", "no", "partial"]),
    ("Which container owns a port", ["yes", "no", "no", "no"]),
    ("Disk usage, biggest offenders", ["yes", "no", "no", "partial"]),
    ("JSON with derived fields", ["yes", "raw only", "no", "no"]),
    ("Themes: palette, glyphs, layout", ["yes", "no", "no", "no"]),
    ("Start, stop, exec, remove", ["no, by design", "no", "no", "yes"]),
]


def cast_size(name: str) -> tuple[int, int]:
    return svg_size(DOCS / "img" / name)


def cast_img(cast: str, still: str, alt: str, extra: str = "") -> str:
    """An animated cast that respects reduced motion and can be paused.

    The casts are SVG with SMIL animation inside an <img>, which neither CSS
    nor script can pause. So motion is controlled by choosing the file: the
    still frame under prefers-reduced-motion, and on demand via data-still.
    """
    w, h = cast_size(cast)
    return (
        f'<picture><source srcset="/img/{still}" media="(prefers-reduced-motion: reduce)">'
        f'<img src="/img/{cast}" data-still="/img/{still}" width="{w}" height="{h}" alt="{html.escape(alt)}"{extra}></picture>'
    )


def home_page(ver: str, today: str) -> str:
    faq = json.loads((ROOT / "scripts/site-faq.json").read_text())
    themes = json.loads(run("themes", "--demo", "--json"))["themes"]
    calls = api_calls()
    subs = [k for _, ks in ORDER for k in ks]

    # the jq example is computed, not typed: real JSON, real filter
    ports = json.loads(run("ports", "--demo", "--json"))["ports"]
    hit = [p for p in ports if p["host_port"] == 5432][0]
    jq_cmd = "dok ports --json | jq '.ports[] | select(.host_port == 5432)'"

    title = "dok: a readable docker ps, images, inspect, logs and disk usage"
    desc = ("dok is a Rust CLI that makes Docker output readable: containers grouped by compose "
            "project, which container owns a port, image layers, disk usage and logs, with JSON for scripts.")
    keywords = [
        "docker ps alternative", "readable docker ps", "pretty docker ps", "docker cli tool",
        "docker which container is using port", "docker disk space", "docker system prune dry run",
        "docker inspect readable", "docker logs multiple containers", "docker image layer size",
        "docker healthcheck failing", "docker ps json", "eza for docker", "lazydocker alternative",
        "rust docker cli", "compose project containers",
    ]
    ld = [
        {
            "@context": "https://schema.org", "@type": "SoftwareApplication", "name": "dok",
            "applicationCategory": "DeveloperApplication", "applicationSubCategory": "Command Line Utility",
            "operatingSystem": "macOS, Linux, Windows", "description": desc, "url": f"{SITE}/",
            "downloadUrl": "https://github.com/alsaadii98/dok/releases",
            "codeRepository": "https://github.com/alsaadii98/dok", "programmingLanguage": "Rust",
            "license": "https://opensource.org/licenses/MIT", "softwareVersion": ver,
            "offers": {"@type": "Offer", "price": "0", "priceCurrency": "USD"},
            "author": {"@type": "Person", "name": "alsaadii98", "url": "https://github.com/alsaadii98"},
        },
        {
            "@context": "https://schema.org", "@type": "FAQPage",
            "mainEntity": [{"@type": "Question", "name": q, "acceptedAnswer": {"@type": "Answer", "text": a}} for q, a in faq],
        },
        {
            "@context": "https://schema.org", "@type": "WebSite", "name": "dok", "url": f"{SITE}/",
        },
    ]
    ld_html = "".join(f'<script type="application/ld+json">{json.dumps(x, ensure_ascii=False)}</script>\n' for x in ld)

    hw, hh = cast_size("cast-hero.svg")

    stages = [
        ("socket", "docker.sock", "or DOCKER_HOST, or a named pipe"),
        ("client", "bollard", "the Docker HTTP API, in Rust"),
        ("views", "src/dk.rs", "names, compose labels, health"),
        ("layout", "src/table.rs", "measure, cap, shrink, pad"),
        ("paint", "src/theme.rs", "roles, glyphs, layout"),
        ("out", "your terminal", "or a pipe, or --json"),
    ]
    pipe = "".join(
        f'<li style="--i:{i}"><span class="st">{s}</span><code translate="no">{html.escape(c)}</code><span class="dsc">{html.escape(d)}</span></li>'
        for i, (s, c, d) in enumerate(stages)
    )

    tabs, panels = [], []
    for i, k in enumerate(BROWSE):
        about = parse_help(run(k, "--help"))["about"]
        shot = COMMANDS[k]["shot"]
        cast = f"cast-{k}.svg" if (DOCS / "img" / f"cast-{k}.svg").exists() else shot
        w, h = cast_size(cast)
        sel = "true" if i == 0 else "false"
        al = f'<span class="al">{ALIAS[k]}</span>' if k in ALIAS else ""
        tabs.append(
            f'<button role="tab" id="t-{k}" data-key="{k}" aria-selected="{sel}" aria-controls="p-{k}" tabindex="{0 if i == 0 else -1}">{k}{al}</button>'
        )
        panels.append(
            f'<div class="panel" role="tabpanel" id="p-{k}" aria-labelledby="t-{k}"{"" if i == 0 else " hidden"}>'
            f'<div class="head"><p>{md(about)}</p><a class="link" href="/man/{k}/">dok-{k}(1) &rarr;</a></div>'
            + cast_img(f"cast-{k}.svg", f"frame-{k}.svg", f"Output of dok {k}", ' loading="lazy" decoding="async"') + "</div>"
        )

    inst_tabs, inst_panels = [], []
    for i, (key, label, cmd, note) in enumerate(INSTALL):
        sel = "true" if i == 0 else "false"
        inst_tabs.append(
            f'<button role="tab" id="it-{key}" data-key="{key}" aria-selected="{sel}" aria-controls="{key}" tabindex="{0 if i == 0 else -1}">{label}</button>'
        )
        lines = "\n".join(f'<span class="p">$ </span>{html.escape(l)}' for l in cmd.split("\n"))
        inst_panels.append(
            f'<div class="panel" role="tabpanel" id="{key}" aria-labelledby="it-{key}"{"" if i == 0 else " hidden"}>'
            f'<div class="code"><button class="copy" type="button" data-copy="{html.escape(cmd)}" aria-label="Copy {label} install commands">copy</button>'
            f'<pre><code translate="no">{lines}</code></pre></div><p>{html.escape(note)}</p></div>'
        )

    cards = []
    for t in themes:
        p = t["palette"]
        sw = "".join(f'<span style="background:{p[r]}"></span>' for r in ["fg", "gray", "red", "orange", "yellow", "green", "cyan", "blue", "magenta"])
        cards.append(
            f'<article class="theme"><h3>{html.escape(t["name"])}</h3><p>{html.escape(t["description"])}</p>'
            f'<div class="sw" role="img" aria-label="{html.escape(t["name"])} palette">{sw}</div>'
            f'<code class="use" translate="no">dok ps --theme {html.escape(t["name"])}</code></article>'
        )

    rows = "".join(
        f'<tr><th scope="row">{html.escape(a)}</th>' + "".join(
            f'<td class="{"y" if v.startswith("yes") and j == 0 else ""}">{html.escape(v)}</td>' for j, v in enumerate(vals)
        ) + "</tr>"
        for a, vals in COMPARE
    )

    faq_html = "".join(
        f'<details><summary>{md(q)}</summary><div class="a prose"><p>{md(a)}</p></div></details>' for q, a in faq
    )

    hit_json = highlight(json.dumps(hit, indent=2))

    # Each one-liner carries its caption as a shell comment inside its own box,
    # so the column holds only boxes and can be stretched to the left block's
    # exact top and bottom.
    one_liners = [
        ("every service that is not healthy",
         "dok health --json | jq -r '.checks[] | select(.status != \"healthy\") | .container'"),
        ("how much a prune would free", "dok prune --json | jq -r .total.human"),
        ("only stderr, as it arrives", "dok logs -f --json | jq -c 'select(.stream == \"stderr\")'"),
    ]
    side_html = "".join(
        f'<div class="code"><button class="copy" type="button" data-copy="{html.escape(c)}" aria-label="Copy command">copy</button>'
        f'<pre><code translate="no"><span class="c"># {html.escape(n)}</span>\n<span class="p">$ </span>{html.escape(c)}</code></pre></div>'
        for n, c in one_liners
    )

    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<meta name="description" content="{html.escape(desc)}">
<meta name="keywords" content="{html.escape(", ".join(keywords))}">
<meta name="author" content="alsaadii98">
<meta name="robots" content="index, follow, max-image-preview:large">
<meta name="theme-color" content="#070708">
<meta name="color-scheme" content="dark">
<link rel="canonical" href="{SITE}/">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<meta property="og:type" content="website">
<meta property="og:site_name" content="dok">
<meta property="og:title" content="dok: Docker output, made readable">
<meta property="og:description" content="What eza is to ls, dok is to Docker. Compose grouping, port ownership, image layers, disk usage and JSON for scripts. One Rust binary.">
<meta property="og:url" content="{SITE}/">
<meta property="og:image" content="{SITE}/img/dok-social.png">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="dok ps output: containers grouped by compose project with state, health, ports and ages">
<meta property="og:locale" content="en_US">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="dok: Docker output, made readable">
<meta name="twitter:description" content="What eza is to ls, dok is to Docker. One Rust binary, read-only, with JSON for scripts.">
<meta name="twitter:image" content="{SITE}/img/dok-social.png">
<link rel="preload" href="/fonts/GeistMono-Variable.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="/fonts/Geist-Variable.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="/img/cast-hero.svg" as="image" fetchpriority="high">
<link rel="stylesheet" href="/assets/site.css">
<link rel="stylesheet" href="/assets/home.css">
<script src="/assets/site.js" defer></script>
{ld_html}</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<div class="atmos" aria-hidden="true"></div>
<p id="live" class="sr" aria-live="polite"></p>
{nav("home")}
<main id="main">
<div class="wrap" id="top">
  <div class="hero">
    <div>
      <h1><span class="ln">Docker output,</span><span class="ln">made <span class="dec" data-decode>readable</span>.</span></h1>
      <p class="lede">What <a href="https://eza.rocks/">eza</a> is to <code>ls</code>, dok is to Docker: containers, images, logs and disk usage, printed for a person to read.</p>
      <div class="cta">
        <div class="cmdline"><span class="p">$</span><code translate="no">brew install alsaadii98/tap/dok</code><button class="copy" type="button" data-copy="brew install alsaadii98/tap/dok" aria-label="Copy install command">copy</button></div>
        <a class="link" href="/man/">Read the manual &rarr;</a>
      </div>
    </div>
    <div class="shotframe">
      {cast_img("cast-hero.svg", "frame-hero.svg", "dok ps -a, dok images, dok logs and dok events rendering in a terminal", ' fetchpriority="high"')}
      <button class="pause" type="button" data-pause aria-pressed="false">Pause animations</button>
    </div>
  </div>
</div>

<section id="how" aria-labelledby="how-h">
  <div class="wrap">
    <h2 id="how-h" class="reveal">One socket. No shell-out.</h2>
    <p class="lede">dok talks to the Docker API directly and lays out every table itself. It only ever reads, so it is safe to point at production.</p>
    <ol class="pipe" data-pipeline>{pipe}</ol>
    <p class="facts"><span><b>{len(calls)}</b> Docker API calls, all reads</span><span><b>0</b> calls to the docker CLI</span><span><b>{len(subs)}</b> commands</span><a class="link" href="/man/internals/">How it works, in detail &rarr;</a></p>
  </div>
</section>

<section id="commands" aria-labelledby="cmd-h">
  <div class="wrap">
    <h2 id="cmd-h" class="reveal">Read, never write.</h2>
    <p class="lede">Every command prints one screen and exits, so it pipes into <code>grep</code> and <code>less</code> like anything else. Pick one to see it run.</p>
    <div class="browser" data-tabs data-hash="cmd-">
      <div role="tablist" aria-label="Commands" aria-orientation="vertical">{"".join(tabs)}</div>
      <div>{"".join(panels)}</div>
    </div>
  </div>
</section>

<section id="json" aria-labelledby="json-h">
  <div class="wrap">
    <h2 id="json-h" class="reveal">Pipe it anywhere.</h2>
    <p class="lede"><code>--json</code> works on every command and carries what docker does not give you: the compose project, the service name, the health verdict.</p>
    <div class="jsonsplit">
      <div class="code"><button class="copy" type="button" data-copy="{html.escape(jq_cmd)}" aria-label="Copy command">copy</button><pre><code translate="no"><span class="p">$ </span>{html.escape(jq_cmd)}
{hit_json}</code></pre></div>
      <div class="side-list">{side_html}</div>
      </div>
    </div>
  </div>
</section>

<section id="themes" aria-labelledby="th-h">
  <div class="wrap">
    <h2 id="th-h" class="reveal">{len(themes)} themes. Palette, glyphs and layout.</h2>
    <p class="lede">A theme changes more than colour: its glyph set and table layout too, so <code>ascii</code> stays pure ASCII for CI logs. Start from any of them in <code>~/.config/dok/config.toml</code>.</p>
    <div class="strip" role="region" tabindex="0" aria-label="Built-in themes">{"".join(cards)}</div>
  </div>
</section>

<section id="install" aria-labelledby="in-h">
  <div class="wrap">
    <h2 id="in-h" class="reveal">{len(INSTALL)} ways in.</h2>
    <p class="lede">One static binary, no runtime. It needs a reachable Docker daemon and honours <code>DOCKER_HOST</code>. Try it with no daemon at all: every command takes <code>--demo</code>.</p>
    <div class="inst" data-tabs data-hash="install-">
      <div role="tablist" aria-label="Install method">{"".join(inst_tabs)}</div>
      {"".join(inst_panels)}
    </div>
  </div>
</section>

<section id="update" aria-labelledby="up-h">
  <div class="wrap upd">
    <h2 id="up-h" class="reveal">It tells you when it is old.</h2>
    <p class="lede">Once a day at most, dok checks for a release and prints one dim line after its output. Never when piped, never with <code>--json</code>. A packaged install gets its package manager&#8217;s command instead of an overwrite.</p>
    <div class="code"><button class="copy" type="button" data-copy="dok update" aria-label="Copy command">copy</button><pre><code translate="no"><span class="c">dok {ver} is out, run `dok update`</span>

<span class="p">$ </span>dok update --check   <span class="c"># report only</span>
<span class="p">$ </span>dok update           <span class="c"># install, after asking</span></code></pre></div>
  </div>
</section>

<section id="compare" aria-labelledby="cmp-h">
  <div class="wrap">
    <h2 id="cmp-h" class="reveal">For reading, not driving.</h2>
    <p class="lede">lazydocker is the right tool when you want to drive containers. dok is for when you want to read them.</p>
    <div class="tbl">
      <table>
        <thead><tr><th scope="col"><span class="sr">Capability</span></th><th scope="col">dok</th><th scope="col">docker ps --format</th><th scope="col">dops</th><th scope="col">lazydocker / oxker</th></tr></thead>
        <tbody>{rows}</tbody>
      </table>
    </div>
  </div>
</section>

<section id="faq" aria-labelledby="faq-h">
  <div class="wrap">
    <h2 id="faq-h" class="reveal">Questions</h2>
    <div class="faq">{faq_html}</div>
  </div>
</section>
</main>
{FOOT}
</body>
</html>
"""


if __name__ == "__main__":
    main()
