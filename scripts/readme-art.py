"""Draws the README's picture cards (docs/readme/*.svg) and the website's bird.

The bird is the app icon sampled into a grid (B = bird, S = the blue square): run
`powershell -File app/src-tauri/windows/icon-grid.ps1` after changing the icon, then `python scripts/readme-art.py`.
Numbers in the cards are real measurements: update them when they change.
"""
import html
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "docs", "readme")
GRID = os.path.join(ROOT, "app", "src-tauri", "windows", "icon-grid.txt")
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace"
SANS = "-apple-system, 'Segoe UI', Helvetica, Arial, sans-serif"
CODE = ("struct ContentView: View { @State private var count = 0 var body: some View { VStack(spacing: 16) "
        "{ Text(\"Hello\").font(.largeTitle).bold() Button(\"Tap me\") { count += 1 } .buttonStyle(.borderedProminent) "
        "} } } swiftwin run --device usb ")

BG, LINE, TEXT, DIM, BLUE, CYAN = "#0b0d12", "#23262d", "#f2f4f8", "#8b93a1", "#2f81f7", "#7cc4ff"


def esc(s):
    return html.escape(s, quote=True)


def bird(x, y, size=9.6, step_y=11.4):
    """The icon as code characters: the bird bright, the square dim."""
    rows = open(GRID, encoding="ascii").read().splitlines()
    out, k = [], 0
    for r, row in enumerate(rows):
        bright, dim = [], []
        for ch in row:
            c = CODE[k % len(CODE)]
            k += 1
            c = "." if c == " " else c
            bright.append(esc(c) if ch == "B" else " ")
            dim.append(esc(c) if ch == "S" else " ")
        ty = y + r * step_y
        out.append(f'<text x="{x}" y="{ty}" fill="{BLUE}" fill-opacity=".22" xml:space="preserve">{"".join(dim)}</text>')
        out.append(f'<text x="{x}" y="{ty}" fill="#ffffff" font-weight="700" xml:space="preserve">{"".join(bright)}</text>')
    return f'<g font-family="{MONO}" font-size="{size}">' + "".join(out) + "</g>"


def cursor(x, y, h=40, w=16):
    return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{CYAN}">'
            '<animate attributeName="opacity" values="1;1;0;0" dur="1.1s" repeatCount="indefinite"/></rect>')


def card(w, h, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">'
            f'<rect x=".5" y=".5" width="{w-1}" height="{h-1}" rx="14" fill="{BG}" stroke="{LINE}"/>{body}</svg>\n')


def hero():
    stats = [("0", "Macs needed"), ("23 fps", "live mirror"), ("4 s", "rebuild"), ("20+", "AI tools")]
    s = [bird(34, 52),
         f'<text x="34" y="30" font-family="{MONO}" font-size="12" fill="{DIM}">~/swiftwin/0.1</text>',
         f'<circle cx="436" cy="92" r="4" fill="{CYAN}"/>',
         f'<text x="448" y="96" font-family="{MONO}" font-size="12" letter-spacing="2" fill="{DIM}">OPEN SOURCE · WINDOWS 10/11</text>',
         f'<text x="432" y="160" font-family="{MONO}" font-size="56" font-weight="700" fill="{TEXT}">SwiftWin</text>',
         cursor(708, 118, 46, 22),
         f'<text x="434" y="196" font-family="{SANS}" font-size="19" fill="{DIM}">SwiftUI iPhone apps, built and run from Windows.</text>']
    for i, (n, label) in enumerate(stats):
        x = 434 + i * 122
        s.append(f'<text x="{x}" y="262" font-family="{MONO}" font-size="26" font-weight="700" fill="{TEXT}">{esc(n)}</text>')
        s.append(f'<text x="{x}" y="284" font-family="{MONO}" font-size="11" letter-spacing="1" fill="{DIM}">{esc(label.upper())}</text>')
    s.append(f'<text x="434" y="352" font-family="{MONO}" font-size="10.5" fill="{DIM}" fill-opacity=".8">'
             'Measured on a recent iPhone (iOS 27) and Windows 11. Rebuild = compile after an edit.</text>')
    return card(960, 380, "".join(s))


def numbers():
    rows = [("First build (once, cold cache)", 301, "301 s", "#4b5260"),
            ("Edit → running on the iPhone", 27, "27 s", TEXT),
            ("Compile after an edit", 4, "4 s", CYAN)]
    s = [f'<text x="40" y="52" font-family="{MONO}" font-size="12" letter-spacing="3" fill="{DIM}">BENCHMARK · REAL IPHONE · WINDOWS 11</text>',
         f'<text x="40" y="98" font-family="{MONO}" font-size="28" font-weight="700" fill="{TEXT}">Edit. Save. It\'s on your phone.</text>']
    for i, (label, v, txt, color) in enumerate(rows):
        y = 150 + i * 56
        w = max(6, 560 * v / 301)
        s.append(f'<text x="40" y="{y+14}" font-family="{MONO}" font-size="13" fill="{TEXT}">{esc(label)}</text>')
        s.append(f'<rect x="330" y="{y}" width="560" height="18" rx="3" fill="#161a21"/>')
        s.append(f'<rect x="330" y="{y}" width="{w:.0f}" height="18" rx="3" fill="{color}"/>')
        s.append(f'<text x="{330 + w + 10:.0f}" y="{y+14}" font-family="{MONO}" font-size="13" fill="{DIM}">{esc(txt)}</text>')
    s.append(f'<text x="40" y="338" font-family="{MONO}" font-size="11" fill="{DIM}">'
             'Then the mirror: 23 fps over USB, and every click is a tap on the real phone.</text>')
    return card(960, 370, "".join(s))


def sponsor():
    s = [f'<text x="40" y="64" font-family="{MONO}" font-size="26" font-weight="700" fill="{TEXT}">Like SwiftWin?</text>',
         cursor(270, 40, 30, 15),
         f'<rect x="660" y="34" width="262" height="44" rx="22" fill="none" stroke="{TEXT}" stroke-width="1.5"/>',
         f'<text x="791" y="61" text-anchor="middle" font-family="{MONO}" font-size="15" font-weight="700" fill="{TEXT}">Sponsor the project ♥</text>']
    return card(960, 112, "".join(s))


os.makedirs(OUT, exist_ok=True)
for name, svg in (("hero", hero()), ("numbers", numbers()), ("sponsor", sponsor())):
    open(os.path.join(OUT, name + ".svg"), "w", encoding="utf-8", newline="\n").write(svg)
print("wrote", OUT)
