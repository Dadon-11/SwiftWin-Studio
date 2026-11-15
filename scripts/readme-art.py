"""Draws the README's picture cards (docs/readme/*-light.svg and *-dark.svg) in the website's style:
frosted glass panels over soft color. Run `python scripts/readme-art.py` after changing them.
Numbers in the cards are real measurements: update them when they change.
"""
import base64
import html
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "docs", "readme")
ICON = os.path.join(ROOT, "app", "src-tauri", "icons", "128x128.png")
SANS = "-apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI', Helvetica, Arial, sans-serif"

THEMES = {
    "light": dict(bg="#f5f5f7", text="#1d1d1f", dim="#6e6e73", accent="#0071e3", glass="#ffffff", glass_op=".62",
                  line="#ffffff", line_op=".9", edge="#000000", edge_op=".06", track="#000000", track_op=".06",
                  blobs=("#7cc4ff", "#a78bfa", "#5eead4"), blob_op=".55"),
    "dark": dict(bg="#000000", text="#f5f5f7", dim="#a1a1a6", accent="#2997ff", glass="#28282e", glass_op=".55",
                 line="#ffffff", line_op=".14", edge="#ffffff", edge_op=".06", track="#ffffff", track_op=".08",
                 blobs=("#1d4ed8", "#6d28d9", "#0e7490"), blob_op=".6"),
}


def esc(s):
    return html.escape(s, quote=True)


def svg(w, h, t, body, blobs):
    """A card: background, blurred color blobs, then the content."""
    b1, b2, b3 = t["blobs"]
    aura = "".join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{c}" opacity="{t["blob_op"]}"/>'
                   for (x, y, r), c in zip(blobs, (b1, b2, b3)))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
            '<defs><filter id="blur" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="60"/></filter>'
            f'<clipPath id="r"><rect width="{w}" height="{h}" rx="28"/></clipPath></defs>'
            f'<g clip-path="url(#r)"><rect width="{w}" height="{h}" fill="{t["bg"]}"/><g filter="url(#blur)">{aura}</g>{body}</g>'
            f'<rect x=".5" y=".5" width="{w-1}" height="{h-1}" rx="28" fill="none" stroke="{t["edge"]}" stroke-opacity="{t["edge_op"]}"/>'
            '</svg>\n')


def glass(t, x, y, w, h, r=22):
    """A frosted panel: translucent fill, a bright top edge and a hairline border."""
    return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{t["glass"]}" fill-opacity="{t["glass_op"]}" '
            f'stroke="{t["line"]}" stroke-opacity="{t["line_op"]}"/>'
            f'<rect x="{x + r}" y="{y + .5}" width="{w - 2 * r}" height="1" fill="#ffffff" opacity=".35"/>')


def text(x, y, s, size, color, weight=400, anchor="middle", spacing=None):
    ls = f' letter-spacing="{spacing}"' if spacing else ""
    return (f'<text x="{x}" y="{y}" font-family="{SANS}" font-size="{size}" font-weight="{weight}" fill="{color}" '
            f'text-anchor="{anchor}"{ls}>{esc(s)}</text>')


def hero(t, icon):
    w, h = 960, 440
    s = [f'<image href="data:image/png;base64,{icon}" x="{w/2 - 48}" y="44" width="96" height="96"/>',
         text(w / 2, 196, "SwiftWin Studio", 52, t["text"], 700, spacing="-1.5"),
         text(w / 2, 236, "iPhone apps. Built on Windows.", 24, t["dim"], 600, spacing="-.4")]
    stats = [("0", "Macs needed"), ("4 s", "to compile an edit"), ("27 s", "edit to iPhone"), ("23 fps", "live mirror")]
    cw, gap = 192, 16
    x0 = (w - (4 * cw + 3 * gap)) / 2
    for i, (n, label) in enumerate(stats):
        x = x0 + i * (cw + gap)
        s.append(glass(t, x, 286, cw, 108))
        s.append(text(x + cw / 2, 340, n, 34, t["text"], 700, spacing="-1"))
        s.append(text(x + cw / 2, 370, label, 14, t["dim"]))
    return svg(w, h, t, "".join(s), ((120, 40, 220), (860, 220, 200), (420, 470, 220)))


def numbers(t):
    w, h = 960, 330
    rows = [("First build, once", 301, "301 s", t["dim"]),
            ("Edit → running on iPhone", 27, "27 s", t["accent"]),
            ("Compile after an edit", 4, "4 s", t["accent"])]
    s = [text(w / 2, 70, "Fast enough to forget about.", 34, t["text"], 700, spacing="-1"),
         text(w / 2, 102, "Measured on a recent iPhone with iOS 27 and Windows 11.", 16, t["dim"]),
         glass(t, 60, 132, w - 120, 162)]
    for i, (label, v, val, color) in enumerate(rows):
        y = 172 + i * 42
        bw = max(10, 440 * v / 301)
        s.append(text(96, y + 5, label, 16, t["text"], 500, anchor="start"))
        s.append(f'<rect x="360" y="{y - 8}" width="440" height="14" rx="7" fill="{t["track"]}" fill-opacity="{t["track_op"]}"/>')
        s.append(f'<rect x="360" y="{y - 8}" width="{bw:.0f}" height="14" rx="7" fill="{color}"/>')
        s.append(text(864, y + 5, val, 16, t["dim"], 600, anchor="end"))
    return svg(w, h, t, "".join(s), ((80, 300, 200), (900, 40, 200), (500, 160, 120)))


def sponsor(t):
    w, h = 960, 150
    s = [glass(t, 24, 24, w - 48, h - 48),
         text(72, 84, "Like SwiftWin?", 28, t["text"], 700, anchor="start", spacing="-.6"),
         text(72, 108, "Sponsoring keeps it free and open source.", 15, t["dim"], anchor="start"),
         f'<rect x="{w - 72 - 190}" y="51" width="190" height="48" rx="24" fill="{t["accent"]}"/>',
         text(w - 72 - 95, 81, "Sponsor ♥", 17, "#ffffff", 600)]
    return svg(w, h, t, "".join(s), ((120, 160, 160), (850, -20, 160), (500, 200, 120)))


icon = base64.b64encode(open(ICON, "rb").read()).decode()
os.makedirs(OUT, exist_ok=True)
for name in ("hero", "numbers", "sponsor"):
    for theme, t in THEMES.items():
        body = hero(t, icon) if name == "hero" else numbers(t) if name == "numbers" else sponsor(t)
        open(os.path.join(OUT, f"{name}-{theme}.svg"), "w", encoding="utf-8", newline="\n").write(body)
print("wrote", OUT)
