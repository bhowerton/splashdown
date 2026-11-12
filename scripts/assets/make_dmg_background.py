# Generates app/src-tauri/dmg/background.tiff, the picture behind the two
# icons in the disk image's window: an arrow from the app to the
# Applications folder and the words "Drag Splashdown to Applications".
#
#   python3 scripts/assets/make_dmg_background.py
#   python3 scripts/assets/make_dmg_background.py --preview /tmp/dmg-preview
#
# Needs rsvg-convert (librsvg) and macOS's own tiffutil and sips.
#
# How the disk image uses it: the bundler copies the file named by
# `bundle.macOS.dmg.background` in tauri.conf.json into the image as it is
# (bundle_dmg.sh, `--background`), and Finder shows it at its size in points.
# Nothing makes a sharper copy for Retina screens on the way, so the file
# itself carries both: a TIFF with a 660 x 400 picture at 72 dpi and a
# 1320 x 800 one at 144 dpi (tiffutil -cathidpicheck). Finder picks the one
# that suits the screen.
#
# Finder draws the icons' names itself, black in light appearance and white
# in dark, straight onto this picture. So the band they sit on is a middle
# blue, about as far from black as from white: both stay readable. The
# script checks that before it writes anything.
#
# --preview also writes light.png and dark.png: the picture with the icons
# and their names drawn where Finder puts them, to check the layout without
# making a disk image.
import base64
import os
import subprocess
import sys
import tempfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "app", "src-tauri", "dmg", "background.tiff")
APP_ICON = os.path.join(ROOT, "app", "src-tauri", "icons", "128x128@2x.png")
FOLDER_ICON = "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/ApplicationsFolderIcon.icns"

# Must match bundle.macOS.dmg in tauri.conf.json. Positions are the icons'
# centres; the bundler leaves the icon size at 128 and the names at 16.
W, H = 660, 400
APP_AT = (180, 170)
FOLDER_AT = (480, 170)
ICON = 128
LABEL = 16
# Finder's window bounds include its title bar, so about this much of the
# picture's bottom edge falls outside the window: keep words above it.
HIDDEN_BOTTOM = 28

# The app icon's colours (site/style.css).
NIGHT, DEEP, DAWN = "#0a1440", "#16337a", "#2f64b8"
ORANGE, ORANGE_LIGHT = "#f2551f", "#ff9a3d"
# The middle blue behind the icons' names (see the check below).
MID = "#4174c6"
FONT = "Helvetica Neue, Helvetica, Arial, sans-serif"


def luminance(hex_colour):
    def channel(c):
        c = c / 255
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4

    r, g, b = (int(hex_colour[i : i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast(a, b):
    hi, lo = max(a, b), min(a, b)
    return (hi + 0.05) / (lo + 0.05)


# Both black and white names must reach 4.5:1 on the band they sit on.
LUM = luminance(MID)
ON_BLACK, ON_WHITE = contrast(LUM, 0.0), contrast(LUM, 1.0)
if min(ON_BLACK, ON_WHITE) < 4.5:
    sys.exit(f"{MID}: names would be hard to read ({ON_BLACK:.2f}:1 black, {ON_WHITE:.2f}:1 white)")


def arrow():
    """A thick arrow between the icons, a little curved, in the canopy's orange."""
    x0 = APP_AT[0] + ICON // 2 + 22
    x1 = FOLDER_AT[0] - ICON // 2 - 22
    y = APP_AT[1]
    mid = (x0 + x1) / 2
    head = 26
    shaft = f"M{x0},{y + 4} Q{mid},{y - 22} {x1 - head + 6},{y}"
    tip = f"M{x1 - head},{y - 17} L{x1 + 2},{y + 1} L{x1 - head + 2},{y + 18} Z"
    return (
        f'<g filter="url(#lift)">'
        f'<path d="{shaft}" fill="none" stroke="url(#canopy)" stroke-width="11" stroke-linecap="round"/>'
        f'<path d="{tip}" fill="{ORANGE}" stroke="{ORANGE}" stroke-width="6" stroke-linejoin="round"/>'
        f"</g>"
    )


def stars():
    """A few small stars in the top corners, clear of the words and icons."""
    at = [(40, 34, 1.6), (92, 112, 1.1), (604, 40, 1.4), (560, 108, 1.0), (628, 132, 1.2), (24, 150, 1.0)]
    return "".join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="#fff" opacity=".7"/>' for x, y, r in at)


def waves():
    """The sea at the bottom, as on the icon: two soft lines and a darker band."""
    y = H - HIDDEN_BOTTOM - 46
    return (
        f'<path d="M0,{y} C110,{y - 12} 220,{y + 12} 330,{y} S550,{y - 12} 660,{y} V{H} H0 Z" fill="url(#sea)"/>'
        f'<path d="M200,{y + 22} Q330,{y + 12} 460,{y + 22}" fill="none" stroke="#fff" stroke-opacity=".28" stroke-width="2" stroke-linecap="round"/>'
        f'<path d="M250,{y + 34} Q330,{y + 27} 410,{y + 34}" fill="none" stroke="#fff" stroke-opacity=".2" stroke-width="2" stroke-linecap="round"/>'
    )


def background_svg(extra=""):
    title_y = 64
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{DEEP}"/>
    <stop offset=".3" stop-color="{DAWN}"/>
    <stop offset=".52" stop-color="{MID}"/>
  </linearGradient>
  <linearGradient id="sea" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{DAWN}"/>
    <stop offset="1" stop-color="{DEEP}"/>
  </linearGradient>
  <linearGradient id="canopy" x1="0" y1="0" x2="1" y2="0">
    <stop offset="0" stop-color="{ORANGE_LIGHT}"/>
    <stop offset="1" stop-color="{ORANGE}"/>
  </linearGradient>
  <filter id="lift" x="-20%" y="-50%" width="140%" height="200%">
    <feDropShadow dx="0" dy="2" stdDeviation="2.5" flood-color="{NIGHT}" flood-opacity=".45"/>
  </filter>
</defs>
<rect width="{W}" height="{H}" fill="url(#sky)"/>
{stars()}
{waves()}
<text x="{W / 2}" y="{title_y}" text-anchor="middle" font-family="{FONT}" font-size="27" font-weight="bold" fill="#fff" filter="url(#lift)">Drag Splashdown to Applications</text>
<text x="{W / 2}" y="{H - HIDDEN_BOTTOM - 70}" text-anchor="middle" font-family="{FONT}" font-size="15" fill="#fff" fill-opacity=".9">Then open it from your Applications folder.</text>
{arrow()}
{extra}
</svg>
"""


def data_uri(png_path):
    with open(png_path, "rb") as f:
        return "data:image/png;base64," + base64.b64encode(f.read()).decode()


def preview_extra(tmp, ink):
    """The two icons and their names, roughly as Finder draws them."""
    folder_png = os.path.join(tmp, "folder.png")
    subprocess.run(["sips", "-s", "format", "png", "-z", "256", "256", FOLDER_ICON, "--out", folder_png],
                   check=True, capture_output=True)
    parts = []
    for (x, y), png, name in ((APP_AT, APP_ICON, "Splashdown"), (FOLDER_AT, folder_png, "Applications")):
        parts.append(f'<image x="{x - ICON // 2}" y="{y - ICON // 2}" width="{ICON}" height="{ICON}" href="{data_uri(png)}"/>')
        parts.append(f'<text x="{x}" y="{y + ICON // 2 + LABEL + 2}" text-anchor="middle" font-family="{FONT}" '
                     f'font-size="{LABEL}" fill="{ink}">{name}</text>')
    # Where the window's visible area ends.
    parts.append(f'<rect x="0" y="{H - HIDDEN_BOTTOM}" width="{W}" height="{HIDDEN_BOTTOM}" fill="#000" opacity=".55"/>')
    return "".join(parts)


def render(svg, png, scale):
    subprocess.run(["rsvg-convert", "-z", str(scale), "-o", png], input=svg.encode(), check=True)


def main():
    preview = None
    if "--preview" in sys.argv:
        preview = sys.argv[sys.argv.index("--preview") + 1]
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        one, two = os.path.join(tmp, "background.png"), os.path.join(tmp, "background@2x.png")
        render(background_svg(), one, 1)
        render(background_svg(), two, 2)
        # tiffutil reads each picture's size in points from its dpi.
        subprocess.run(["sips", "-s", "dpiWidth", "72", "-s", "dpiHeight", "72", one], check=True, capture_output=True)
        subprocess.run(["sips", "-s", "dpiWidth", "144", "-s", "dpiHeight", "144", two], check=True, capture_output=True)
        subprocess.run(["tiffutil", "-cathidpicheck", one, two, "-out", OUT], check=True)
        print(f"wrote {os.path.relpath(OUT, ROOT)}")
        if preview:
            os.makedirs(preview, exist_ok=True)
            for name, ink in (("light", "#000"), ("dark", "#fff")):
                out = os.path.join(preview, f"{name}.png")
                render(background_svg(preview_extra(tmp, ink)), out, 2)
                print(f"wrote {out}")


if __name__ == "__main__":
    main()
