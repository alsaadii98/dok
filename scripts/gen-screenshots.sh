#!/usr/bin/env bash
# Regenerate the SVGs used by README.md and the website.
#
# Everything renders from `--demo`, dok's built-in example stack, so the docs
# show real rendering without leaking whatever runs on the machine that
# generated them. No daemon required.
#
# Two flavours come out of the same captured output:
#   docs/img/*.svg       static frames, used by README.md (GitHub-safe)
#   docs/img/cast-*.svg  animated: types the command, then reveals the output
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p docs/img

BIN=${DOK_BIN:-target/release/dok}
[ -x "$BIN" ] || cargo build --release

RAW=$(mktemp -d)
trap 'rm -rf "$RAW"' EXIT

# Fixed flags keep the SVGs reproducible regardless of the caller's terminal.
capture() {
  local name=$1
  shift
  "$BIN" "$@" --demo --color=always --icons=unicode >"$RAW/$name.ansi"
}

capture ps       ps -a
capture images   images
capture df       df -v --top 3
capture df-slim  df
capture tree     tree
capture tree-p   tree --only projects
capture logs     logs -n 8
capture themes   themes
capture inspect  inspect api
capture top      top api
capture events   events --since 20m

# Static frames for the README.
still() {
  python3 scripts/ansi2svg.py --out "docs/img/$1.svg" --title "$2" <"$RAW/$1.ansi"
}
still ps      "dok ps -a"
still images  "dok images"
still df      "dok df -v"
still tree    "dok tree"
still logs    "dok logs -n 8"
still themes  "dok themes"
still inspect "dok inspect api"

# Animated casts for the website.
cast() {
  local out=$1 title=$2
  shift 2
  local scenes=()
  for spec in "$@"; do
    scenes+=(--scene "$spec")
  done
  python3 scripts/ansi2cast.py --out "docs/img/cast-$out.svg" --title "$title" "${scenes[@]}"
}

# The hero cycles through four commands in one file. They are picked to be
# roughly the same height, so the frame does not sit half-empty between scenes.
cast hero "dok — demo stack" \
  "dok ps -a=$RAW/ps.ansi" \
  "dok images=$RAW/images.ansi" \
  "dok logs -n 8=$RAW/logs.ansi" \
  "dok events --since 20m=$RAW/events.ansi"

cast ps      "dok ps"      "dok ps -a=$RAW/ps.ansi"
cast images  "dok images"  "dok images=$RAW/images.ansi"
cast df      "dok df"      "dok df=$RAW/df-slim.ansi"
cast tree    "dok tree"    "dok tree --only projects=$RAW/tree-p.ansi"
cast logs    "dok logs"    "dok logs -n 8=$RAW/logs.ansi"
cast inspect "dok inspect" "dok inspect api=$RAW/inspect.ansi"
cast top     "dok top"     "dok top api=$RAW/top.ansi"
cast events  "dok events"  "dok events --since 20m=$RAW/events.ansi"
cast themes  "dok themes"  "dok themes=$RAW/themes.ansi"

echo "screenshots written to docs/img/"
