# Generates the five small animations in the first-run setup guide, one per
# physical step people get wrong: which switch to turn on, which key to hold,
# and where the app's icon is. Each is 560x240, transparent, 4 to 6 s, and
# looks right on a dark or a light page.
#
#   python3 scripts/assets/make_setup.py            # writes app/ui/img/setup-*.svg
#   python3 scripts/assets/make_setup.py some/dir   # or somewhere else
#
# How each file is built: the plain, unanimated drawing IS the end state
# (switch on, key held, menu open, icon on the taskbar). The keyframes only
# describe how it gets there, so a reader who asks for reduced motion just
# has the animation switched off and sees the finished picture.
import sys
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "app" / "ui" / "img"

FONT = "system-ui,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif"

# Colours as variables, in two sets: each picture is written twice, for
# light pages and (name-dark.svg) for dark ones, and the page shows the one
# that suits it. A rule inside the picture can't do that by itself: shown
# with <img>, a picture isn't told the page is dark in every browser engine
# (the app's own, on a Mac, is one that doesn't).
# The accent matches the app's own (base.css).
LIGHT = """:root{--panel:#fff;--screen:#EEF1F7;--bar:#fff;--line:#CFD3DC;--text:#1D1E23;--grey:#C3C7D1;--grey2:#DDE0E7;
--track:#B4B9C5;--accent:#0A6CF0;--soft:#DCE9FD;--key:#fff;--keyside:#C9CDD7;--body:#E2E5EC;--hover:#E3E6EE;--sh:.22}"""
DARK = """:root{--panel:#2B2C32;--screen:#25272F;--bar:#1B1C21;--line:#4A4C56;--text:#ECECEF;--grey:#555864;--grey2:#42444E;
--track:#5E616D;--accent:#3C8BFF;--soft:#1F3A6B;--key:#3A3C45;--keyside:#202127;--body:#2C2E36;--hover:#3A3C46;--sh:.5}"""
THEME = """
@media (prefers-reduced-motion:reduce){*{animation:none!important}}
text{font-family:%s;fill:var(--text)}
.panel{fill:var(--panel);stroke:var(--line);stroke-width:1.5}
.screen{fill:var(--screen);stroke:var(--line);stroke-width:2}
.bar{fill:var(--bar)}
.hair{stroke:var(--line);stroke-width:1.5;fill:none}
.g1{fill:var(--grey)}.g2{fill:var(--grey2)}
.ink{stroke:var(--text);fill:none;stroke-width:1.7;stroke-linecap:round;stroke-linejoin:round}
.inkf{fill:var(--text)}
.acc{fill:var(--accent)}.accs{fill:var(--accent);stroke:none}
.accline{stroke:var(--accent);fill:none}
.body{fill:var(--body);stroke:var(--line);stroke-width:1.5}
.key{fill:var(--key);stroke:var(--line);stroke-width:1.5}
.side{fill:var(--keyside)}
.tk{fill:var(--soft);stroke:var(--accent);stroke-width:2.5}
.soft{fill:var(--soft)}
.wh{fill:#fff}
""" % FONT

# Shared drawings, defined once and reused. The app icon is the real icon's
# shapes, simplified (no stars, shadow or ripples) so it stays clear small.
DEFS = """
<linearGradient id="sky" x1="0" y1="100" x2="0" y2="760" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#0A1440"/><stop offset=".6" stop-color="#16337A"/><stop offset="1" stop-color="#2F64B8"/></linearGradient>
<linearGradient id="sea" x1="0" y1="730" x2="0" y2="924" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#0F6FA8"/><stop offset="1" stop-color="#06305C"/></linearGradient>
<linearGradient id="org" x1="0" y1="190" x2="0" y2="450" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#FF9A3D"/><stop offset="1" stop-color="#F2551F"/></linearGradient>
<path id="plate" d="M390 100H634C866 100 924 158 924 390V634C924 866 866 924 634 924H390C158 924 100 866 100 634V390C100 158 158 100 390 100Z"/>
<clipPath id="pc"><use href="#plate"/></clipPath>
<path id="canopy" d="M232 440C232 250 360 186 512 186C664 186 792 250 792 440Q756 462 702 440Q640 466 578 440Q512 470 446 440Q384 466 322 440Q268 462 232 440Z"/>
<clipPath id="cc"><use href="#canopy"/></clipPath>
<symbol id="app" viewBox="100 100 824 824">
<use href="#plate" fill="url(#sky)"/>
<g clip-path="url(#pc)">
<g stroke="#fff" stroke-opacity=".8" stroke-width="14" stroke-linecap="round"><path d="M240 446L480 578"/><path d="M784 446L544 578"/></g>
<g clip-path="url(#cc)"><rect x="220" y="180" width="584" height="300" fill="#fff"/>
<path d="M512 186Q232 186 232 470H330Q340 230 512 186Z" fill="url(#org)"/><path d="M512 186Q452 230 446 470H578Q572 230 512 186Z" fill="url(#org)"/><path d="M512 186Q792 186 792 470H694Q684 230 512 186Z" fill="url(#org)"/></g>
<g transform="translate(512 742) scale(1.2) translate(-512 -742)"><rect x="486" y="584" width="52" height="26" rx="8" fill="#9FB0C9"/><path d="M470 606H554Q564 606 568 616L620 728Q512 760 404 728L456 616Q460 606 470 606Z" fill="#F1F5FB"/><circle cx="512" cy="640" r="24" fill="#16337A"/></g>
<path d="M100 742Q190 726 280 742T460 742T640 742T820 742T1000 742V940H100Z" fill="url(#sea)"/>
<g stroke="#fff" stroke-width="26" stroke-linecap="round" fill="none"><path d="M370 738Q334 690 296 680"/><path d="M654 738Q690 690 728 680"/></g>
<path d="M366 748Q512 782 658 748Q646 766 512 776Q378 766 366 748Z" fill="#fff"/>
</g></symbol>
<symbol id="glyph" viewBox="0 0 36 36"><mask id="gm" maskUnits="userSpaceOnUse" x="0" y="0" width="36" height="36"><rect width="36" height="36" fill="#fff"/><g stroke="#000" stroke-width="1.4" stroke-linecap="round" fill="none"><path d="M16 3.2Q13.2 6 13 14"/><path d="M20 3.2Q22.8 6 23 14"/></g></mask>
<g fill="currentColor"><path mask="url(#gm)" d="M3 12.5C3 5.5 9.8 1.5 18 1.5C26.2 1.5 33 5.5 33 12.5Q28 15 23 12.5Q18 15 13 12.5Q8 15 3 12.5Z"/><path d="M15.4 20.6H20.6L24.4 27.4Q18 29 11.6 27.4Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/></g>
<g stroke="currentColor" stroke-width="1.8" stroke-linecap="round" fill="none"><path d="M4 13.6L15.2 20.8"/><path d="M32 13.6L20.8 20.8"/><path d="M4 32.2Q7.5 29.4 11 32.2T18 32.2T25 32.2T32 32.2" stroke-width="2.2"/></g></symbol>
<symbol id="ptr" viewBox="-2 -2 18 24" overflow="visible"><path d="M0 0V17L4.4 13.2L7.2 19.8L10 18.6L7.2 12.2H12.8Z" fill="#fff" stroke="#14151A" stroke-width="1.5" stroke-linejoin="round"/></symbol>
<filter id="sh" x="-20%" y="-20%" width="140%" height="150%"><feDropShadow dx="0" dy="3" stdDeviation="4" flood-color="#000" flood-opacity=".22"/></filter>
"""


class Svg:
    """One picture: collects keyframes and shapes, then writes the file."""

    def __init__(self, name, title, dur):
        self.name, self.title, self.dur = name, title, dur
        self.css, self.n, self.parts = [], 0, []

    def anim(self, frames, dur=None, timing="ease-in-out", delay=0):
        """Keyframes from [("0,40", "opacity:0"), ...] (percent stops); returns a class."""
        self.n += 1
        n = f"a{self.n}"
        body = " ".join(
            f"{','.join(s + '%' for s in stops.split(','))}{{{css}}}" for stops, css in frames)
        d = dur or self.dur
        self.css.append(
            f"@keyframes {n}{{{body}}}.{n}{{animation:{n} {d}s {timing} {delay}s infinite}}")
        return n

    def add(self, s):
        self.parts.append(s)

    def write(self):
        for colours, suffix in ((LIGHT, ""), (DARK, "-dark")):
            svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="560" height="240" viewBox="0 0 560 240" role="img">'
                   f'<title>{escape(self.title)}</title><defs>{DEFS}</defs>'
                   f'<style>{colours}{THEME}\n' + "\n".join(self.css) + '</style>'
                   + "".join(self.parts) + '</svg>\n')
            name = self.name.replace(".svg", f"{suffix}.svg")
            (OUT / name).write_text(svg)
            print(f"{name}: {len(svg.encode()) / 1024:.1f} KB")


def pointer(s, frames, dur=None):
    """The mouse pointer, tip at the origin, moved by keyframes. Hidden when still."""
    c = s.anim(frames, dur)
    s.add(f'<g class="{c}" style="opacity:0"><use href="#ptr" width="18" height="24" '
          f'transform="scale(1.15)" overflow="visible"/></g>')


def ring_pop(s, cx, cy, start, end, r0=8, r1=24, dur=None):
    """A ring that grows and fades where something was just clicked."""
    c = s.anim([(f"0,{start}", f"transform:scale(.4);opacity:0"),
                (f"{start + 1}", "transform:scale(.4);opacity:.7"),
                (f"{end}", "transform:scale(1);opacity:0"),
                (f"{end + 1},100", "transform:scale(1);opacity:0")], dur, "ease-out")
    s.add(f'<circle class="accline {c}" cx="{cx}" cy="{cy}" r="{r1}" stroke-width="3" '
          f'style="opacity:0;transform-box:fill-box;transform-origin:center"/>')


# ------------------------------------------------------------ 1: accessibility
def accessibility():
    s = Svg("setup-accessibility.svg", "Turning on the Splashdown switch in a settings list", 5.6)
    CX, CY, CW, CH = 70, 14, 420, 212
    s.add(f'<g filter="url(#sh)"><rect class="panel" x="{CX}" y="{CY}" width="{CW}" height="{CH}" rx="16"/></g>')
    ROW, TOP = 48, 22
    # grey, unnamed rows around the one that matters
    names = [None, "Splashdown", None, None]
    bars = [118, 0, 96, 134]
    for i, nm in enumerate(names):
        y = CY + TOP + i * ROW - 6
        cy = y + 24
        if nm:
            s.add(f'<rect class="soft" x="{CX + 8}" y="{y - 2}" width="{CW - 16}" height="{ROW - 2}" rx="11"/>')
            s.add(f'<use href="#app" x="{CX + 22}" y="{cy - 16}" width="32" height="32"/>')
            s.add(f'<text x="{CX + 66}" y="{cy + 6.5}" font-size="19" font-weight="700">Splashdown</text>')
        else:
            s.add(f'<g opacity=".55"><rect class="g1" x="{CX + 22}" y="{cy - 16}" width="32" height="32" rx="8"/>'
                  f'<rect class="g2" x="{CX + 66}" y="{cy - 6}" width="{bars[i]}" height="12" rx="6"/></g>')
        # the switch: off at the start, on at the end
        sx = CX + CW - 22 - 52
        if nm:
            t = s.anim([("0,45", "fill:var(--track)"), ("49,100", "fill:var(--accent)")])
            k = s.anim([("0,45", "transform:translateX(0)"), ("49,100", "transform:translateX(22px)")])
            s.add(f'<rect class="{t}" x="{sx}" y="{cy - 15}" width="52" height="30" rx="15" style="fill:var(--accent)"/>')
            s.add(f'<circle class="wh {k}" cx="{sx + 15}" cy="{cy}" r="11.5" style="transform:translateX(22px)"/>')
            swx, swy = sx + 26, cy
        else:
            s.add(f'<g opacity=".55"><rect style="fill:var(--track)" x="{sx}" y="{cy - 15}" width="52" height="30" rx="15"/>'
                  f'<circle class="wh" cx="{sx + 15}" cy="{cy}" r="11.5"/></g>')
    ring_pop(s, swx, swy, 46, 60, dur=None)
    # the pointer comes in from lower right, clicks, holds, leaves
    pointer(s, [("0", f"transform:translate(420px,214px);opacity:0"),
                ("6", "transform:translate(420px,214px);opacity:1"),
                ("38,45", f"transform:translate({swx + 2}px,{swy + 3}px) scale(1);opacity:1"),
                ("47", f"transform:translate({swx + 2}px,{swy + 3}px) scale(.82);opacity:1"),
                ("52,86", f"transform:translate({swx + 2}px,{swy + 3}px) scale(1);opacity:1"),
                ("92,100", f"transform:translate(430px,222px);opacity:0")])
    s.write()


# ------------------------------------------------------------------ keyboards
def keyboard(s, spec, right, target, label_kind):
    """The bottom-right corner of a keyboard. spec is a list of (x, y, w, h, drawer);
    keys are laid out in a left-to-right row and shifted so the body ends 12px
    from the picture's right edge. `target` is the index of the key to hold."""
    ox = 548 - right
    s.add(f'<rect class="body" x="-60" y="92" width="{right + ox + 60}" height="112" rx="22"/>')
    for i, (x, y, w, h, draw) in enumerate(spec):
        if i == target:
            continue
        x += ox
        s.add(f'<rect class="side" x="{x}" y="{y + 3}" width="{w}" height="{h}" rx="9"/>'
              f'<rect class="key" x="{x}" y="{y}" width="{w}" height="{h}" rx="9"/>')
        s.add(draw(x, y, w, h))
    # the held key: a halo that pulses, the key sunk and tinted, a label, a level line
    x, y, w, h, draw = spec[target]
    x += ox
    cx = x + w / 2
    P1, P2, R1, R2 = 10, 14, 84, 88
    vis = s.anim([(f"0,{P1}", "opacity:0"), (f"{P2},{R1}", "opacity:1"), (f"{R2},100", "opacity:0")])
    pulse = s.anim([("0", "opacity:.22;transform:scale(1)"), ("50", "opacity:.45;transform:scale(1.06)"),
                    ("100", "opacity:.22;transform:scale(1)")], dur=1.3)
    s.add(f'<g class="{vis}"><rect class="accs {pulse}" x="{x - 7}" y="{y - 5}" width="{w + 14}" height="{h + 14}" rx="15" '
          f'style="opacity:.4;transform-box:fill-box;transform-origin:center"/></g>')
    down = s.anim([(f"0,{P1}", "transform:translateY(0)"), (f"{P2},{R1}", "transform:translateY(3px)"),
                   (f"{R2},100", "transform:translateY(0)")])
    tint = s.anim([(f"0,{P1}", "fill:var(--key)"), (f"{P2},{R1}", "fill:var(--soft)"), (f"{R2},100", "fill:var(--key)")])
    s.add(f'<rect class="side" x="{x}" y="{y + 3}" width="{w}" height="{h}" rx="9"/>')
    s.add(f'<g class="{down}" style="transform:translateY(3px)"><rect class="tk {tint}" x="{x}" y="{y}" width="{w}" height="{h}" rx="9"/>'
          + draw(x, y, w, h) + '</g>')
    # "hold" with a level line either side: speak while the key is down
    py = 50
    s.add(f'<g class="{vis}" style="opacity:1">')
    s.add(f'<rect class="acc" x="{cx - 36}" y="{py}" width="72" height="30" rx="15"/>'
          f'<text x="{cx}" y="{py + 21}" text-anchor="middle" font-size="17" font-weight="700" style="fill:#fff">hold</text>')
    heights = [12, 24, 36, 22, 14]
    for side in (-1, 1):
        for j in range(5):
            hh = heights[j if side == 1 else 4 - j]
            bx = cx + side * (54 + j * 12) - 2.5
            c = s.anim([("0", "transform:scaleY(.3)"), ("50", "transform:scaleY(1)"), ("100", "transform:scaleY(.3)")],
                       dur=0.8, delay=-(j * 0.17 + (0.1 if side < 0 else 0)))
            s.add(f'<rect class="accs {c}" x="{bx}" y="{py + 15 - hh / 2}" width="5" height="{hh}" rx="2.5" '
                  f'style="transform-box:fill-box;transform-origin:center"/>')
    s.add('</g>')


def txt(x, y, t, size, weight=600, anchor="middle"):
    return f'<text x="{x}" y="{y}" font-size="{size}" font-weight="{weight}" text-anchor="{anchor}">{escape(t)}</text>'


def option_sign(cx, cy):
    # The option sign drawn as lines, so it never depends on a font having it.
    return (f'<path class="ink" style="stroke-width:2.4" d="M{cx - 10} {cy - 7}H{cx - 4}L{cx + 4} {cy + 7}H{cx + 10}M{cx + 2} {cy - 7}H{cx + 10}"/>')


def arrow_key(d):
    def draw(x, y, w, h):
        cx, cy = x + w / 2, y + h / 2
        pts = {"l": f"{cx + 4},{cy - 6} {cx - 5},{cy} {cx + 4},{cy + 6}", "r": f"{cx - 4},{cy - 6} {cx + 5},{cy} {cx - 4},{cy + 6}",
               "u": f"{cx - 6},{cy + 3} {cx},{cy - 4} {cx + 6},{cy + 3}", "d": f"{cx - 6},{cy - 3} {cx},{cy + 4} {cx + 6},{cy - 3}"}[d]
        return f'<polygon class="inkf" points="{pts}" style="opacity:.75"/>'
    return draw


def arrows(x0, y, kw, kh=58, gap=6):
    half = (kh - gap) / 2
    return [(x0, y, kw, kh, arrow_key("l")),
            (x0 + kw + gap, y, kw, half, arrow_key("u")),
            (x0 + kw + gap, y + half + gap, kw, half, arrow_key("d")),
            (x0 + 2 * (kw + gap), y, kw, kh, arrow_key("r"))]


def nothing(x, y, w, h):
    return ""


def key_mac():
    s = Svg("setup-key-mac.svg", "Holding the right Option key on a Mac keyboard while you speak", 5.2)
    Y = 112

    def cmd(x, y, w, h):
        return txt(x + w / 2, y + 25, "⌘", 17, 500) + txt(x + w / 2, y + 46, "command", 11.5, 500)

    def opt(x, y, w, h):
        return option_sign(x + w / 2, y + 22) + txt(x + w / 2, y + 47, "option", 16, 700)
    spec = [(-170, Y, 330, 58, nothing), (166, Y, 66, 58, cmd), (238, Y, 66, 58, opt)] + arrows(310, Y, 46)
    keyboard(s, spec, 310 + 3 * 46 + 12 + 8, 2, "mac")
    s.write()


def key_win():
    s = Svg("setup-key-win.svg", "Holding the right Ctrl key on a PC keyboard while you speak", 5.2)
    Y = 112

    def altgr(x, y, w, h):
        return txt(x + w / 2, y + 35, "Alt Gr", 13, 500)

    def menu(x, y, w, h):
        cx, cy = x + w / 2, y + h / 2
        return (f'<rect class="ink" style="stroke-width:1.5" x="{cx - 10}" y="{cy - 9}" width="20" height="18" rx="3"/>'
                f'<path class="ink" style="stroke-width:1.5" d="M{cx - 5} {cy - 3}H{cx + 5}M{cx - 5} {cy + 3}H{cx + 5}"/>')

    def ctrl(x, y, w, h):
        return txt(x + w / 2, y + 30, "Ctrl", 19, 700) + txt(x + w / 2, y + 47, "right", 11.5, 500)
    spec = [(-170, Y, 332, 58, nothing), (168, Y, 62, 58, altgr), (236, Y, 52, 58, menu), (294, Y, 62, 58, ctrl)] + arrows(362, Y, 46)
    keyboard(s, spec, 362 + 3 * 46 + 12 + 8, 3, "win")
    s.write()


# ------------------------------------------------------------ screen corners
# Plain stand-in status shapes (a wifi fan, a battery, a dot in a ring): none is a real product's icon.
def wifi(cx, cy):
    return (f'<path class="ink" d="M{cx - 9} {cy - 2}Q{cx} {cy - 10} {cx + 9} {cy - 2}M{cx - 5.5} {cy + 2}Q{cx} {cy - 3} {cx + 5.5} {cy + 2}"/>'
            f'<circle class="inkf" cx="{cx}" cy="{cy + 6}" r="1.8"/>')


def battery(cx, cy):
    return (f'<rect class="ink" style="stroke-width:1.5" x="{cx - 12}" y="{cy - 6}" width="22" height="12" rx="3.5"/>'
            f'<rect class="inkf" x="{cx - 9.5}" y="{cy - 3.5}" width="13" height="7" rx="1.6"/>'
            f'<rect class="inkf" x="{cx + 11}" y="{cy - 2.2}" width="2" height="4.4" rx="1"/>')


def dot_ring(cx, cy):
    return f'<circle class="ink" style="stroke-width:1.5" cx="{cx}" cy="{cy}" r="8"/><circle class="inkf" cx="{cx}" cy="{cy}" r="2.8"/>'


def menubar_mac():
    s = Svg("setup-menubar-mac.svg", "The Splashdown icon in the Mac menu bar, and its menu", 6)
    s.add('<defs><clipPath id="scr"><rect x="-60" y="10" width="608" height="300" rx="18"/></clipPath></defs>')
    s.add('<rect class="screen" x="-60" y="10" width="608" height="300" rx="18"/>')
    s.add('<g clip-path="url(#scr)"><rect class="bar" x="-60" y="10" width="608" height="36"/>'
          '<path class="hair" d="M-60 46H548"/></g>')
    cy = 28
    gx = 402
    s.add(dot_ring(342, cy) + battery(436, cy) + wifi(474, cy))
    s.add(f'<rect class="g1" x="296" y="{cy - 6}" width="12" height="12" rx="3"/>')
    s.add(f'<text x="534" y="{cy + 5}" font-size="14" font-weight="600" text-anchor="end">9:41</text>')
    # the app's glyph, with a soft ring that breathes until the menu opens
    ringvis = s.anim([("0,2", "opacity:0"), ("8,94", "opacity:1"), ("99,100", "opacity:0")])
    breathe = s.anim([("0", "transform:scale(1)"), ("50", "transform:scale(1.14)"), ("100", "transform:scale(1)")], dur=1.5)
    s.add(f'<g class="{ringvis}"><circle class="accs {breathe}" cx="{gx}" cy="{cy}" r="22" '
          f'style="opacity:.2;transform-box:fill-box;transform-origin:center"/>'
          f'<circle class="accline" cx="{gx}" cy="{cy}" r="20" stroke-width="2.5"/></g>')
    s.add(f'<use href="#glyph" x="{gx - 13}" y="{cy - 13}" width="26" height="26" style="color:var(--text)"/>')
    # the menu: plain grey lines and one you can read
    MX, MY, MW = 352, 54, 196
    menu = s.anim([("0,39", "opacity:0;transform:translateY(-6px)"), ("45,92", "opacity:1;transform:none"),
                   ("97,100", "opacity:0;transform:translateY(-6px)")])
    hov = s.anim([("0,52", "fill:transparent"), ("56,100", "fill:var(--accent)")])
    htx = s.anim([("0,52", "fill:var(--text)"), ("56,100", "fill:#fff")])
    s.add(f'<g class="{menu}"><g filter="url(#sh)"><rect class="panel" x="{MX}" y="{MY}" width="{MW}" height="140" rx="12"/></g>')
    for yy, bw in ((72, 110), (100, 78)):
        s.add(f'<rect class="g2" x="{MX + 16}" y="{yy - 6}" width="{bw}" height="12" rx="6"/>')
    s.add(f'<path class="hair" d="M{MX + 12} 118H{MX + MW - 12}"/>')
    s.add(f'<rect class="{hov}" x="{MX + 8}" y="124" width="{MW - 16}" height="30" rx="8" style="fill:var(--accent)"/>')
    s.add(f'<text class="{htx}" x="{MX + 20}" y="145" font-size="16" font-weight="600" style="fill:#fff">Settings…</text>')
    s.add(f'<rect class="g2" x="{MX + 16}" y="{170 - 6}" width="92" height="12" rx="6"/></g>')
    s.write()


def tray_win():
    s = Svg("setup-tray-win.svg", "Dragging the Splashdown icon out of the hidden icons onto the Windows taskbar", 6)
    s.add('<defs><clipPath id="scr"><rect x="-60" y="-70" width="608" height="300" rx="18"/></clipPath></defs>')
    s.add('<rect class="screen" x="-60" y="-70" width="608" height="300" rx="18"/>')
    s.add('<g clip-path="url(#scr)"><rect class="bar" x="-60" y="182" width="608" height="50"/><path class="hair" d="M-60 182H548"/></g>')
    cy = 206
    UX, UY = 340, cy                    # the "^"
    SLOT = (460, cy)                    # where the icon ends up
    # "^" gets a pad that lights up when clicked
    pad = s.anim([("0,24", "fill:transparent"), ("28,40", "fill:var(--hover)"), ("46,100", "fill:transparent")])
    s.add(f'<rect class="{pad}" x="{UX - 17}" y="{cy - 17}" width="34" height="34" rx="8" style="fill:transparent"/>')
    s.add(f'<path class="ink" style="stroke-width:2.2" d="M{UX - 7} {cy + 3}L{UX} {cy - 4}L{UX + 7} {cy + 3}"/>')
    s.add(wifi(384, cy) + f'<path class="ink" d="M{412} {cy - 3}H{416}L{421} {cy - 7}V{cy + 7}L{416} {cy + 3}H{412}Z"/>'
          f'<path class="ink" d="M{426} {cy - 4}Q{430} {cy} {426} {cy + 4}"/>')
    s.add(f'<text x="515" y="{cy + 5}" font-size="14" font-weight="600" text-anchor="middle">9:41</text>')
    ring_pop(s, UX, UY, 24, 38, dur=None)
    # the panel of hidden icons, opening above the arrow
    PX, PY, PW, PH = 296, 70, 176, 102
    cells = [(PX + 40, PY + 33), (PX + 88, PY + 33), (PX + 136, PY + 33),
             (PX + 40, PY + 77 - 3), (PX + 88, PY + 77 - 3), (PX + 136, PY + 77 - 3)]
    cells[0], cells[1] = cells[1], cells[0]      # the app sits in the middle of the top row
    panel = s.anim([("0,28", "opacity:0;transform:translateY(8px)"), ("34,70", "opacity:1;transform:none"),
                    ("76,100", "opacity:0;transform:translateY(8px)")])
    s.add(f'<g class="{panel}" style="opacity:0"><g filter="url(#sh)"><rect class="panel" x="{PX}" y="{PY}" width="{PW}" height="{PH}" rx="12"/></g>')
    shapes = [None, "sq", "ci", "ci", "sq", "tri"]
    for (x, y), sh in zip(cells, shapes):
        if sh is None:
            continue
        if sh == "sq":
            s.add(f'<rect class="g1" x="{x - 14}" y="{y - 14}" width="28" height="28" rx="7" opacity=".75"/>')
        elif sh == "ci":
            s.add(f'<circle class="g1" cx="{x}" cy="{y}" r="14" opacity=".75"/>')
        else:
            s.add(f'<rect class="g1" x="{x - 14}" y="{y - 14}" width="28" height="28" rx="14" opacity=".75"/>')
    ax, ay = cells[0]
    # the icon in its cell, dimmed once picked up
    dim = s.anim([("0,48", "opacity:1"), ("52,100", "opacity:.25")])
    s.add(f'<use class="{dim}" href="#app" x="{ax - 17}" y="{ay - 17}" width="34" height="34" style="opacity:.25"/>')
    s.add('</g>')
    # the icon being carried: same path as the pointer, shrinking to taskbar size
    ex, ey = SLOT
    carry = s.anim([("0,51", f"opacity:0;transform:translate({ax}px,{ay}px) scale(1)"),
                    ("52", f"opacity:1;transform:translate({ax}px,{ay}px) scale(1)"),
                    ("72", f"opacity:1;transform:translate({ex}px,{ey}px) scale(.7)"),
                    ("73,100", f"opacity:0;transform:translate({ex}px,{ey}px) scale(.7)")])
    s.add(f'<g class="{carry}" style="opacity:0"><use href="#app" x="-17" y="-17" width="34" height="34"/></g>')
    # the icon at rest on the taskbar, beside the clock
    rest = s.anim([("0,72", "opacity:0"), ("74,96", "opacity:1"), ("99,100", "opacity:0")])
    s.add(f'<use class="{rest}" href="#app" x="{ex - 12}" y="{ey - 12}" width="24" height="24"/>')
    ring_pop(s, ex, ey, 72, 84, dur=None)
    pointer(s, [("0", "transform:translate(500px,140px);opacity:0"),
                ("4", "transform:translate(500px,140px);opacity:1"),
                ("22,25", f"transform:translate({UX + 1}px,{UY + 2}px) scale(1);opacity:1"),
                ("27", f"transform:translate({UX + 1}px,{UY + 2}px) scale(.82);opacity:1"),
                ("31", f"transform:translate({UX + 1}px,{UY + 2}px) scale(1);opacity:1"),
                ("46", f"transform:translate({ax + 3}px,{ay + 4}px) scale(1);opacity:1"),
                ("51", f"transform:translate({ax + 3}px,{ay + 4}px) scale(.85);opacity:1"),
                ("72", f"transform:translate({ex + 3}px,{ey + 4}px) scale(.85);opacity:1"),
                ("75", f"transform:translate({ex + 3}px,{ey + 4}px) scale(1);opacity:1"),
                ("90,100", f"transform:translate({ex + 30}px,{ey - 30}px);opacity:0")])
    s.write()


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    accessibility()
    key_mac()
    key_win()
    menubar_mac()
    tray_win()
