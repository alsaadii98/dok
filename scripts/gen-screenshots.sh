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
# Embedding Geist Mono needs fonttools. Without it the SVGs still render,
# in the viewer's own monospace:  pip install fonttools brotli
PY=${PYTHON:-python3}
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
capture ports    ports
capture history  history api
capture health   health
capture prune    prune

# Static frames for the README.
still() {
  "$PY" scripts/ansi2svg.py --out "docs/img/$1.svg" --title "$2" <"$RAW/$1.ansi"
}
still ps      "dok ps -a"
still images  "dok images"
still df      "dok df -v"
still tree    "dok tree"
still logs    "dok logs -n 8"
still themes  "dok themes"
still inspect "dok inspect api"
still ports   "dok ports"
still history "dok history api"
still health  "dok health"
still prune   "dok prune"
# Static frames for the two commands that only had casts, so every animation
# on the site has a still to fall back to under prefers-reduced-motion.
still top     "dok top api"
still events  "dok events --since 20m"

# Animated casts for the website.
cast() {
  local out=$1 title=$2
  shift 2
  local scenes=()
  for spec in "$@"; do
    scenes+=(--scene "$spec")
  done
  "$PY" scripts/ansi2cast.py --out "docs/img/cast-$out.svg" --title "$title" "${scenes[@]}"
}

# The hero cycles through four commands in one file. They are picked to be
# roughly the same height, so the frame does not sit half-empty between scenes.
cast hero "~/demo-shop" \
  "dok ps -a=$RAW/ps.ansi" \
  "dok images=$RAW/images.ansi" \
  "dok logs -n 8=$RAW/logs.ansi" \
  "dok events --since 20m=$RAW/events.ansi"
# and its still, the same size, for the pause control and reduced motion
"$PY" scripts/ansi2cast.py --out docs/img/frame-hero.svg --title "~/demo-shop" --static \
  --scene "dok ps -a=$RAW/ps.ansi" --scene "dok images=$RAW/images.ansi" \
  --scene "dok logs -n 8=$RAW/logs.ansi" --scene "dok events --since 20m=$RAW/events.ansi"

# The command browser on the site shows one cast at a time in one slot, so
# they share a frame: as wide as the widest command's output, and a fixed
# height, like a terminal window. Output that runs longer is cut and marked.
# Each also gets a static twin at the same size, which the pause control and
# reduced motion show instead.
FRAME_ROWS=17
FRAME_COLS=$("$PY" - "$RAW" <<'EOF'
import sys
sys.path.insert(0, "scripts")
from ansi2cast import Scene
raw = sys.argv[1]
specs = [("ps -a","ps"),("ports","ports"),("health","health"),("history api","history"),
         ("prune","prune"),("images","images"),("df","df-slim"),("inspect api","inspect"),
         ("logs -n 8","logs"),("top api","top"),("tree --only projects","tree-p"),
         ("events --since 20m","events"),("themes","themes")]
print(max(Scene("dok " + c, open(f"{raw}/{f}.ansi").read()).cols for c, f in specs))
EOF
)

frame() {
  local name=$1 cmd=$2 file=$3
  "$PY" scripts/ansi2cast.py --out "docs/img/cast-$name.svg" --title "dok $name" \
    --cols "$FRAME_COLS" --rows "$FRAME_ROWS" --scene "dok $cmd=$RAW/$file.ansi"
  "$PY" scripts/ansi2cast.py --out "docs/img/frame-$name.svg" --title "dok $name" \
    --cols "$FRAME_COLS" --rows "$FRAME_ROWS" --static --scene "dok $cmd=$RAW/$file.ansi"
}

frame ps      "ps -a"                ps
frame ports   "ports"                ports
frame health  "health"               health
frame history "history api"          history
frame prune   "prune"                prune
frame images  "images"               images
frame df      "df"                   df-slim
frame inspect "inspect api"          inspect
frame logs    "logs -n 8"            logs
frame top     "top api"              top
frame tree    "tree --only projects" tree-p
frame events  "events --since 20m"   events
frame themes  "themes"               themes

echo "screenshots written to docs/img/"
