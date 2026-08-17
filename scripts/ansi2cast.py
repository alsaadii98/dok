#!/usr/bin/env python3
"""Render ANSI output into an *animated* SVG that types the command first.

Each scene is one command plus the output it produced. The SVG types the
command a character at a time, reveals the output line by line, holds, then
moves to the next scene and loops — all with SMIL, so it animates inside a
plain <img> with no JavaScript.

    scripts/ansi2cast.py --out docs/img/hero.svg \
        --scene "dok ps -a=/tmp/ps.ansi" --scene "dok images=/tmp/images.ansi"

Colours and chrome match the website's cards on purpose: the same card
background, border, radius and text colours, so a cast dropped into a bento
tile reads as part of the page rather than a pasted screenshot.
"""

import argparse
import html
import sys

# Chrome geometry and palette live in ansi2svg, so the static frames and the
# animated casts can never drift apart.
from ansi2svg import BAR, BG, CHAR_W, DOT, FG, HEAD, LINE, LINE_H, MUTED, PAD_X, PAD_Y
from ansi2svg import TITLEBAR, parse

GREEN = "#98c379"

# Timing, in seconds.
LEAD = 0.5  # pause before typing starts
PER_CHAR = 0.05
AFTER_ENTER = 0.4
PER_LINE = 0.055
MAX_REVEAL = 1.5
HOLD = 3.4
FADE = 0.18


def esc(s):
    return html.escape(s, quote=False)


class Scene:
    def __init__(self, cmd, ansi):
        self.cmd = cmd
        self.lines = parse(ansi)

    @property
    def cols(self):
        widest = max((sum(len(t) for t, _ in r) for r in self.lines), default=0)
        return max(widest, len(self.cmd) + 3)

    @property
    def rows(self):
        return len(self.lines) + 2  # prompt line + blank spacer

    @property
    def duration(self):
        typing = LEAD + len(self.cmd) * PER_CHAR + AFTER_ENTER
        reveal = min(len(self.lines) * PER_LINE, MAX_REVEAL)
        return typing + reveal + HOLD


def keytimes(pairs, total):
    """Turn [(seconds, value), ...] into SMIL values/keyTimes strings."""
    vals = ";".join(str(v) for _, v in pairs)
    times = ";".join(f"{min(max(t / total, 0.0), 1.0):.5f}" for t, _ in pairs)
    return vals, times


def anim(attr, pairs, total, calc="linear"):
    vals, times = keytimes(pairs, total)
    return (
        f'<animate attributeName="{attr}" calcMode="{calc}" values="{vals}" '
        f'keyTimes="{times}" dur="{total:.3f}s" repeatCount="indefinite"/>'
    )


def render_scene(scene, start, total, width, idx):
    """One scene's <g>, animated on the global timeline that starts at `start`."""
    out = []
    top = TITLEBAR + PAD_Y
    y_prompt = top + 13
    x_cmd = PAD_X + 2 * CHAR_W  # after "❯ "

    typing_end = start + LEAD + len(scene.cmd) * PER_CHAR
    body_start = typing_end + AFTER_ENTER
    scene_end = start + scene.duration

    # The whole scene fades in and out so scenes never overlap mid-swap.
    gate = [
        (0, 0),
        (max(start - FADE, 0), 0),
        (start, 1),
        (scene_end - FADE, 1),
        (scene_end, 0),
        (total, 0),
    ]
    out.append(f'<g opacity="0">{anim("opacity", gate, total)}')

    # Prompt marker.
    out.append(
        f'<text x="{PAD_X:.1f}" y="{y_prompt:.1f}" fill="{GREEN}" '
        f'font-weight="500">&#10095;</text>'
    )

    # Command text, revealed by a clip that widens one character at a time.
    clip = f"type{idx}"
    steps = [(0, 0), (start + LEAD, 0)]
    for i in range(1, len(scene.cmd) + 1):
        steps.append((start + LEAD + i * PER_CHAR, round(i * CHAR_W, 2)))
    steps.append((total, round(len(scene.cmd) * CHAR_W, 2)))
    out.append(
        f'<clipPath id="{clip}"><rect x="{x_cmd:.1f}" y="0" height="{TITLEBAR + 40:.0f}" '
        f'width="0">{anim("width", steps, total, calc="discrete")}</rect></clipPath>'
    )
    out.append(
        f'<g clip-path="url(#{clip})"><text x="{x_cmd:.1f}" y="{y_prompt:.1f}" '
        f'fill="{HEAD}" xml:space="preserve">{esc(scene.cmd)}</text></g>'
    )

    # Caret: rides the clip edge while typing, then disappears with the output.
    caret = [(0, 0), (start + LEAD - 0.05, 0), (start + LEAD, 1), (body_start, 1),
             (body_start + 0.01, 0), (total, 0)]
    caret_x = [(t, round(x_cmd + w, 2)) for t, w in steps]
    out.append(
        f'<g opacity="0">{anim("opacity", caret, total, calc="discrete")}'
        f'<rect x="{x_cmd:.1f}" y="{y_prompt - 11:.1f}" width="{CHAR_W:.1f}" height="15" '
        f'fill="{HEAD}" opacity=".75">'
        f'{anim("x", caret_x, total, calc="discrete")}'
        f'<animate attributeName="opacity" values=".85;.85;0;0" keyTimes="0;.5;.5;1" '
        f'dur="1s" repeatCount="indefinite"/>'
        f"</rect></g>"
    )

    # Output lines.
    per = min(PER_LINE, MAX_REVEAL / max(len(scene.lines), 1))
    for row, runs in enumerate(scene.lines):
        y = top + (row + 2) * LINE_H + 13
        spans, col = [], 0
        for text, style in runs:
            x = PAD_X + col * CHAR_W
            col += len(text)
            if not text.strip():
                continue
            attrs = [f'x="{x:.1f}"', f'fill="{style.fg or FG}"']
            if style.bold:
                attrs.append('font-weight="600"')
            if style.underline:
                attrs.append('text-decoration="underline"')
            spans.append(f'<tspan {" ".join(attrs)}>{esc(text)}</tspan>')
        if not spans:
            continue
        at = body_start + row * per
        show = [
            (0, 0),
            (at, 0),
            (min(at + 0.14, scene_end), 1),
            (scene_end - FADE, 1),
            (scene_end, 0),
            (total, 0),
        ]
        show = dedupe(show)
        out.append(
            f'<text y="{y:.1f}" opacity="0" xml:space="preserve">'
            f'{anim("opacity", show, total)}{"".join(spans)}</text>'
        )

    out.append("</g>")
    return "\n".join(out)


def dedupe(pairs):
    """SMIL needs strictly non-decreasing keyTimes; clamp any overlap."""
    fixed, last = [], -1.0
    for t, v in pairs:
        t = max(t, last)
        fixed.append((t, v))
        last = t
    return fixed


def render(scenes, title, font):
    cols = max(s.cols for s in scenes)
    rows = max(s.rows for s in scenes)
    width = PAD_X * 2 + cols * CHAR_W
    height = TITLEBAR + PAD_Y * 2 + rows * LINE_H
    total = sum(s.duration for s in scenes)

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" '
        f'viewBox="0 0 {width:.0f} {height:.0f}" font-family="{esc(font)}" font-size="14">',
        f'<rect x=".5" y=".5" width="{width - 1:.0f}" height="{height - 1:.0f}" rx="11.5" '
        f'fill="{BG}" stroke="{LINE}"/>',
        f'<path d="M0 12a12 12 0 0 1 12-12h{width - 24:.0f}a12 12 0 0 1 12 12v{TITLEBAR - 12:.0f}'
        f'H0z" fill="{BAR}"/>',
        f'<path d="M0 {TITLEBAR:.0f}h{width:.0f}" stroke="{LINE}"/>',
    ]
    for i in range(3):
        out.append(f'<circle cx="{22 + i * 16}" cy="{TITLEBAR / 2:.0f}" r="4.5" fill="{DOT}"/>')
    if title:
        out.append(
            f'<text x="{width / 2:.0f}" y="{TITLEBAR / 2 + 4:.0f}" fill="{MUTED}" '
            f'font-size="11.5" text-anchor="middle">{esc(title)}</text>'
        )

    start = 0.0
    for i, scene in enumerate(scenes):
        out.append(render_scene(scene, start, total, width, i))
        start += scene.duration

    out.append("</svg>")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True)
    ap.add_argument("--title", default="")
    ap.add_argument(
        "--scene",
        action="append",
        required=True,
        metavar="CMD=FILE",
        help="command line to type, and the file holding its ANSI output",
    )
    ap.add_argument(
        "--font",
        default="JetBrains Mono, SFMono-Regular, Menlo, Consolas, monospace",
    )
    args = ap.parse_args()

    scenes = []
    for spec in args.scene:
        if "=" not in spec:
            sys.exit(f"--scene needs CMD=FILE, got {spec!r}")
        cmd, path = spec.split("=", 1)
        with open(path, encoding="utf-8") as fh:
            scenes.append(Scene(cmd, fh.read()))

    with open(args.out, "w", encoding="utf-8") as fh:
        fh.write(render(scenes, args.title, args.font) + "\n")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
