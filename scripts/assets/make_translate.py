# Generates docs/assets/translate.svg, the animation for translating:
# first holding Option and tapping T (again for the next language) to
# write an email in French; then, in a German chat, the reply comes out in
# German by itself. One 16 s loop; times are percentages of it.
#
#   python3 scripts/assets/make_translate.py docs/assets/translate.svg
import sys
from html import escape

out = sys.argv[1]
LOOP = 16
F = 0.6
css = ""


def vis(name, start, end):
    """Shown between start and end (in %), faded in and out."""
    global css
    a, b = max(start - F, 0), min(end + F, 100)
    frames = []
    if a > 0:
        frames.append(f"0%,{a:.2f}%{{opacity:0}}")
    frames.append(f"{start:.2f}%,{end:.2f}%{{opacity:1}}")
    if b < 100:
        frames.append(f"{b:.2f}%,100%{{opacity:0}}")
    css += f"@keyframes {name}{{{' '.join(frames)}}}\n.{name}{{animation:{name} {LOOP}s linear infinite}}\n"


def press(name, spans, tint):
    """A key pressed (down and tinted) during each (start, end) span."""
    global css
    down, color = [], []
    last = 0.0
    for s, e in spans:
        down.append(f"{last:.2f}%,{s - 0.1:.2f}%{{transform:none}}{s:.2f}%,{e:.2f}%{{transform:translateY(3px)}}")
        color.append(f"{last:.2f}%,{s - 0.1:.2f}%{{fill:#F4F6FB}}{s:.2f}%,{e:.2f}%{{fill:{tint}}}")
        last = e + 0.1
    down.append(f"{last:.2f}%,100%{{transform:none}}")
    color.append(f"{last:.2f}%,100%{{fill:#F4F6FB}}")
    css += (f"@keyframes press-{name}{{{''.join(down)}}}\n@keyframes tint-{name}{{{''.join(color)}}}\n"
            f".press-{name}{{animation:press-{name} {LOOP}s linear infinite}}"
            f".tint-{name}{{animation:tint-{name} {LOOP}s linear infinite}}\n")


def typing(name, start, end, width, shown_until):
    """A cover over text that slides right like typing, then the text stays
    until `shown_until`."""
    global css
    css += (f"@keyframes cover-{name}{{0%,{start:.2f}%{{transform:none}}{end:.2f}%,100%{{transform:translateX({width}px)}}}}\n"
            f".cover-{name}{{animation:cover-{name} {LOOP}s linear infinite}}\n")
    vis(f"typed-{name}", start, shown_until)


RED, GREEN, ORANGE = "#F04545", "#52CF66", "#FF9A3D"
LABEL = 'fill="#F4F4F5" font-size="17" font-weight="600"'
HINT = 'fill="#F4F4F5" fill-opacity=".6" font-size="13"'
INK, MUTED, BLUE = "#111827", "#6B7280", "#2563EB"

# ------------------------------------------------------------------ timing
# Scene A, 0–48: an email; hold ⌥, T (Spanish), T again (French), speak.
vis("scene-a", 0.6, 48)
vis("pa-listen", 4, 8)
vis("pa-es", 8.6, 14)
vis("pa-fr", 14.6, 33)
vis("pa-work", 33.6, 37.4)
vis("pa-done", 38, 47)
vis("say-a", 17, 33)
typing("a", 38, 44, 262, 47.5)
vis("cur-a", 4, 38)
press("opt", [(4, 33), (54, 72)], "#FFD5D5")
press("t", [(8, 9.4), (14, 15.4)], "#FFE2C2")
vis("hint-t", 8.6, 33)

# Scene B, 52–100: a German chat; hold ⌥ and the reply comes out in German.
vis("scene-b", 50.6, 99.4)
vis("pb-listen", 54, 56)
vis("pb-de", 56.6, 72)
vis("pb-work", 72.6, 76.4)
vis("pb-done", 77, 98)
vis("say-b", 58, 72)
typing("b", 77, 85, 412, 99)
vis("cur-b", 54, 77)
vis("hint-their", 56.6, 72)


def wave(x0, y, color):
    return "<g>" + "".join(
        f'<rect class="bar" x="{x0 + i * 10}" y="{y}" width="4" height="22" rx="2" fill="{color}"/>'
        for i in range(6)) + "</g>"


def pill(cls, w, inner, x=None):
    x = (960 - w) / 2 if x is None else x
    return (f'<g class="{cls}"><rect x="{x}" y="330" width="{w}" height="50" rx="25" '
            f'fill="#17171C" fill-opacity=".94" filter="url(#shadow)"/>'
            f'<g transform="translate({x} 0)">{inner}</g></g>')


def check(cx):
    return (f'<path d="M{cx - 7} 355 l5 5 l10 -11" fill="none" stroke="{GREEN}" stroke-width="3" '
            f'stroke-linecap="round" stroke-linejoin="round"/>')


def spinner(cx):
    return (f'<g><circle cx="{cx}" cy="355" r="9" fill="none" stroke="#F4F4F5" stroke-opacity=".25" stroke-width="3"/>'
            f'<path class="spin" d="M{cx} 346 a9 9 0 0 1 9 9" fill="none" stroke="#F4F4F5" stroke-width="3" stroke-linecap="round"/></g>')


def listening(cls, label, w, extra=""):
    return pill(cls, w, f'<circle cx="26" cy="355" r="6" fill="{RED}"/>'
                        f'<text x="41" y="361" {LABEL}>{escape(label)}</text>'
                        + wave(w - 92, 344, RED) + extra)


pills = "".join([
    listening("pa-listen", "Listening", 236),
    listening("pa-es", "→ Spanish", 236),
    listening("pa-fr", "→ French", 236),
    pill("pa-work", 230, spinner(28) + f'<text x="47" y="361" {LABEL}>Translating…</text>'),
    pill("pa-done", 200, check(26) + f'<text x="44" y="361" {LABEL}>Inserted</text><text x="126" y="360" {HINT}>1.1 s</text>'),
    listening("pb-listen", "Listening", 236),
    listening("pb-de", "→ German (their language)", 388),
    pill("pb-work", 230, spinner(28) + f'<text x="47" y="361" {LABEL}>Translating…</text>'),
    pill("pb-done", 200, check(26) + f'<text x="44" y="361" {LABEL}>Inserted</text><text x="126" y="360" {HINT}>1.0 s</text>'),
])

SAY = 'x="480" y="428" text-anchor="middle" font-size="19" font-style="italic" fill="#E6ECFF"'
FIELD = f'font-size="22" fill="{INK}"'


def window(title):
    return (f'<rect x="150" y="40" width="660" height="40" fill="#EEF1F6"/>'
            f'<circle cx="174" cy="60" r="6" fill="#FF5F57"/><circle cx="194" cy="60" r="6" fill="#FEBC2E"/>'
            f'<circle cx="214" cy="60" r="6" fill="#28C840"/>'
            f'<text x="480" y="65" text-anchor="middle" font-size="14" fill="{MUTED}">{escape(title)}</text>')


def cover(name, y):
    return (f'<g class="cover-{name}"><rect x="174" y="{y}" width="620" height="36" fill="#fff"/>'
            f'<rect class="caret" x="174" y="{y + 4}" width="2" height="28" fill="{BLUE}"/></g>')


# Scene A: a new email.
email = (window("New message")
         + f'<text x="176" y="114" font-size="16" fill="#9CA3AF">To:</text>'
         + f'<text x="206" y="114" font-size="16" fill="#374151">Camille Martin</text>'
         + '<line x1="170" y1="130" x2="790" y2="130" stroke="#E5E7EB"/>'
         + f'<g class="typed-a"><text x="176" y="182" {FIELD}>On se voit jeudi à 15 h ?</text>{cover("a", 156)}</g>'
         + f'<g class="cur-a"><rect class="caret" x="176" y="160" width="2" height="28" fill="{BLUE}"/></g>')

# Scene B: a work chat in German, the reply box at the bottom.
chat = (window("#projekt-alpha")
        + f'<rect x="176" y="96" width="34" height="34" rx="8" fill="#6366F1"/>'
        + f'<text x="193" y="119" text-anchor="middle" font-size="15" font-weight="700" fill="#fff">LB</text>'
        + f'<text x="222" y="108" font-size="14" font-weight="700" fill="{INK}">Lukas Becker <tspan font-weight="400" fill="{MUTED}">09:14</tspan></text>'
        + f'<text x="222" y="130" font-size="16" fill="#374151">Kannst du mir die Zahlen bis morgen schicken?</text>'
        + '<rect x="170" y="152" width="620" height="76" rx="10" fill="#fff" stroke="#D1D5DB"/>'
        + f'<g class="typed-b"><text x="186" y="196" {FIELD}>Klar, ich schicke sie dir morgen früh.</text>{cover("b", 170)}</g>'
        + f'<g class="cur-b"><text x="190" y="196" font-size="18" fill="#9CA3AF">Message #projekt-alpha</text>'
        + f'<rect class="caret" x="186" y="174" width="2" height="28" fill="{BLUE}"/></g>')

css += f"""
@keyframes bar{{0%{{transform:scaleY(.25)}}100%{{transform:scaleY(1)}}}}
.bar{{transform-box:fill-box;transform-origin:center;animation:bar .45s ease-in-out infinite alternate}}
.bar:nth-child(2){{animation-delay:-.15s}}.bar:nth-child(3){{animation-delay:-.3s}}
.bar:nth-child(4){{animation-delay:-.05s}}.bar:nth-child(5){{animation-delay:-.25s}}.bar:nth-child(6){{animation-delay:-.4s}}
@keyframes spin{{to{{transform:rotate(360deg)}}}}
.spin{{transform-box:fill-box;transform-origin:center;animation:spin .8s linear infinite}}
@keyframes blink{{0%,49%{{opacity:1}}50%,100%{{opacity:0}}}}
.caret{{animation:blink 1s step-end infinite}}
@keyframes twinkle{{0%,100%{{opacity:.9}}50%{{opacity:.35}}}}
.star{{animation:twinkle 3s ease-in-out infinite}}
.star:nth-of-type(2n){{animation-delay:-1.4s}}.star:nth-of-type(3n){{animation-delay:-2.1s}}

/* Without motion: the German reply, written. */
@media (prefers-reduced-motion:reduce){{
  *{{animation:none!important}}
  [class^="pa-"],[class^="pb-"],.say-a,.say-b,.scene-a,.hint-t,.hint-their,.cover-b,.typed-a,.cur-a,.cur-b{{opacity:0}}
  .scene-b,.typed-b,.pb-done{{opacity:1}}
}}
"""

STARS = "".join(
    f'<circle class="star" cx="{x}" cy="{y}" r="{r}" fill="#fff"/>'
    for x, y, r in [(70, 60, 2.5), (880, 90, 3), (110, 210, 2), (860, 230, 2), (40, 300, 1.8), (920, 330, 2.2)])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="960" height="480" viewBox="0 0 960 480" role="img" aria-labelledby="t d">
<title id="t">Translating as you dictate</title>
<desc id="d">In a new email, holding Option and tapping T shows "→ Spanish"; tapping T again changes it to "→ French". Saying "see you Thursday at 3?" types "On se voit jeudi à 15 h ?". Then, in a work chat where Lukas writes in German, holding Option shows "→ German (their language)" with no key to press, and saying "sure, I'll send them to you tomorrow morning" types "Klar, ich schicke sie dir morgen früh."</desc>
<style>{css}</style>
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="0" y2="480" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0A1440"/><stop offset=".65" stop-color="#16337A"/><stop offset="1" stop-color="#2F64B8"/>
  </linearGradient>
  <linearGradient id="sea" x1="0" y1="440" x2="0" y2="480" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0F6FA8"/><stop offset="1" stop-color="#06305C"/>
  </linearGradient>
  <filter id="shadow" x="-20%" y="-30%" width="140%" height="180%">
    <feDropShadow dx="0" dy="8" stdDeviation="12" flood-color="#000" flood-opacity=".35"/>
  </filter>
  <clipPath id="card"><rect width="960" height="480" rx="28"/></clipPath>
  <clipPath id="window"><rect x="150" y="40" width="660" height="206" rx="14"/></clipPath>
</defs>
<g clip-path="url(#card)" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif">
  <rect width="960" height="480" fill="url(#sky)"/>
  {STARS}
  <path d="M0 452 C160 438 320 466 480 452 S800 438 960 452 V480 H0 Z" fill="url(#sea)"/>

  <!-- The app being dictated into. -->
  <g filter="url(#shadow)"><rect x="150" y="40" width="660" height="206" rx="14" fill="#fff"/></g>
  <g clip-path="url(#window)">
    <g class="scene-a">{email}</g>
    <g class="scene-b">{chat}</g>
  </g>

  <!-- The keys. -->
  <g font-weight="600" font-size="22" fill="#1F2937" text-anchor="middle">
    <g class="press-t"><rect class="tint-t" x="54" y="330" width="52" height="48" rx="10" fill="#F4F6FB"/><text x="80" y="362">T</text></g>
    <g class="press-opt"><rect class="tint-opt" x="118" y="330" width="52" height="48" rx="10" fill="#F4F6FB"/><text x="144" y="361">⌥</text></g>
  </g>
  <text x="80" y="398" text-anchor="middle" font-size="12" fill="#C7D2FE">tap</text>
  <text x="144" y="398" text-anchor="middle" font-size="12" fill="#C7D2FE">hold</text>
  <text class="hint-t" x="830" y="352" text-anchor="middle" font-size="13" fill="#C7D2FE">T again:</text>
  <text class="hint-t" x="830" y="370" text-anchor="middle" font-size="13" fill="#C7D2FE">next language</text>
  <text class="hint-their" x="864" y="352" text-anchor="middle" font-size="13" fill="#C7D2FE">no key to press:</text>
  <text class="hint-their" x="864" y="370" text-anchor="middle" font-size="13" fill="#C7D2FE">the chat is German</text>

  <!-- The overlay pill. -->
  {pills}

  <!-- What the user says. -->
  <text class="say-a" {SAY}>“see you Thursday at 3?”</text>
  <text class="say-b" {SAY}>“sure, I'll send them to you tomorrow morning”</text>
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
