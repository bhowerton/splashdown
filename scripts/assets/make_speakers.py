# Generates docs/assets/speakers.svg, the animation for remembering people
# by voice: in Monday's meeting, Speaker 2 is named Priya once (from the
# people invited); in Thursday's, Priya is known by her voice, and a new
# voice stays Speaker 2. One 18 s loop; times are percentages of it.
#
#   python3 scripts/assets/make_speakers.py docs/assets/speakers.svg
import sys
from html import escape

out = sys.argv[1]
LOOP = 18
F = 0.6
css = ""


def vis(name, start, end, rise=False):
    """Shown between start and end (in %); `rise` slides it up as it comes."""
    global css
    a, b = max(start - F, 0), min(end + F, 100)
    hidden = "opacity:0;transform:translateY(6px)" if rise else "opacity:0"
    shown = "opacity:1;transform:none" if rise else "opacity:1"
    frames = []
    if a > 0:
        frames.append(f"0%,{a:.2f}%{{{hidden}}}")
    frames.append(f"{start:.2f}%,{end:.2f}%{{{shown}}}")
    if b < 100:
        frames.append(f"{b:.2f}%,100%{{{hidden}}}")
    css += f"@keyframes {name}{{{' '.join(frames)}}}\n.{name}{{animation:{name} {LOOP}s linear infinite}}\n"


INK, MUTED, BLUE, THEM = "#111827", "#6B7280", "#2563EB", "#B45309"
MONO = 'font-family="ui-monospace, SFMono-Regular, Menlo, monospace"'
W, H = 960, 480
WY, WW, WH = 40, 420, 320
LX, RX = 40, 500


def frame(x, title, meta):
    return (f'<g filter="url(#shadow)"><rect x="{x}" y="{WY}" width="{WW}" height="{WH}" rx="14" fill="#fff"/></g>'
            f'<rect x="{x}" y="{WY}" width="{WW}" height="36" rx="14" fill="#EEF1F6"/>'
            f'<rect x="{x}" y="{WY + 22}" width="{WW}" height="14" fill="#EEF1F6"/>'
            f'<circle cx="{x + 20}" cy="{WY + 18}" r="5.5" fill="#FF5F57"/><circle cx="{x + 38}" cy="{WY + 18}" r="5.5" fill="#FEBC2E"/>'
            f'<circle cx="{x + 56}" cy="{WY + 18}" r="5.5" fill="#28C840"/>'
            f'<text x="{x + WW / 2}" y="{WY + 23}" text-anchor="middle" font-size="13" fill="{MUTED}">Notes</text>'
            f'<text x="{x + 20}" y="{WY + 66}" font-size="18" font-weight="700" fill="{INK}">{escape(title)}</text>'
            f'<text x="{x + 20}" y="{WY + 86}" font-size="12.5" fill="{MUTED}">{escape(meta)}</text>')


def row(x, i, time, who, said, cls="", who_cls=""):
    y = WY + 120 + i * 50
    color = BLUE if who == "You" else THEM
    who_el = (f'<text class="{who_cls}" x="{x + 66}" y="{y}" font-size="14" font-weight="700" fill="{color}">'
              f'{escape(who)}</text>') if who else ""
    return (f'<g class="{cls}"><text x="{x + 20}" y="{y}" font-size="12" fill="{BLUE}" {MONO}>{time}</text>'
            f'{who_el}<text x="{x + 66}" y="{y + 20}" font-size="14.5" fill="{INK}">{escape(said)}</text></g>')


# ------------------------------------------------------------------ Monday
# Speaker 2 is named Priya: pointer to the label, the menu, the choice,
# every "Speaker 2" becomes "Priya Shah", and the voice is kept.
vis("menu", 16, 23.4)
vis("old", 0, 24.4)
vis("new", 25, 99.4)
vis("toast", 26, 42, rise=True)
vis("say-a", 1, 43)

monday = (frame(LX, "Design review", "Monday · 32 min · With Priya Shah, Dana Ortiz")
          + row(LX, 0, "0:04", "Speaker 1", "Morning! Can we look at onboarding?")
          + row(LX, 1, "0:12", "You", "Sure, I'll share my screen.")
          + row(LX, 2, "0:31", "", "The second step still feels long.")
          + row(LX, 3, "0:48", "", "I'd cut the survey entirely.")
          )
# The second speaker's label, before and after.
for i in (2, 3):
    y = WY + 120 + i * 50
    monday += (f'<text class="old" x="{LX + 66}" y="{y}" font-size="14" font-weight="700" fill="{THEM}">Speaker 2</text>'
               f'<text class="new" x="{LX + 66}" y="{y}" font-size="14" font-weight="700" fill="{THEM}">Priya Shah</text>')
my = WY + 120 + 2 * 50 + 8
monday += (f'<g class="menu"><g filter="url(#soft)"><rect x="{LX + 60}" y="{my}" width="176" height="68" rx="10" fill="#fff" stroke="#E5E7EB"/></g>'
           f'<rect x="{LX + 66}" y="{my + 6}" width="164" height="26" rx="6" fill="#DBEAFE"/>'
           f'<text x="{LX + 76}" y="{my + 24}" font-size="13.5" fill="{INK}">Priya Shah</text>'
           f'<text x="{LX + 76}" y="{my + 54}" font-size="13.5" fill="{MUTED}">Type a name…</text></g>')

# The pointer: to the label, then to "Priya Shah".
label_x, label_y = LX + 100, WY + 120 + 2 * 50 - 5
item_x, item_y = LX + 120, my + 20
css += (f"@keyframes pointer{{0%,8%{{transform:translate({LX + 300}px,{WY + 300}px);opacity:0}}"
        f"9%{{opacity:1}}14%,17%{{transform:translate({label_x}px,{label_y}px);opacity:1}}"
        f"21%,24%{{transform:translate({item_x}px,{item_y}px);opacity:1}}27%,100%{{transform:translate({item_x}px,{item_y}px);opacity:0}}}}\n"
        f".pointer{{animation:pointer {LOOP}s ease-in-out infinite}}\n"
        f"@keyframes click{{0%,15.4%,17.4%,22.4%,24.4%,100%{{transform:scale(1)}}16%,23%{{transform:scale(.85)}}}}\n"
        f".click{{transform-box:fill-box;transform-origin:top left;animation:click {LOOP}s linear infinite}}\n")
pointer = ('<g class="pointer"><path class="click" d="M0 0 L0 18 L5 13.5 L8.5 21 L11.5 19.6 L8 12.4 L14.5 12.4 Z" '
           'fill="#111827" stroke="#fff" stroke-width="1.5" stroke-linejoin="round"/></g>')

# ------------------------------------------------------------------ Thursday
# Priya is known by her voice; a new voice stays Speaker 2.
vis("thursday", 45, 99.4)
vis("say-b", 46, 99)
rows_b = [
    ("0:03", "Priya Shah", "I tried the shorter flow last night.", True),
    ("0:15", "You", "How did it feel?", False),
    ("0:22", "Speaker 2", "Much faster, honestly.", False),
    ("0:40", "Priya Shah", "Let's ship it on Friday.", True),
]
thursday = frame(RX, "Design review", "Thursday · 18 min · With Priya Shah, Sam Lee")
for i, (t, who, said, known) in enumerate(rows_b):
    vis(f"b{i}", 51 + i * 7, 99.4, rise=True)
    badge = ""
    if known:
        y = WY + 120 + i * 50
        badge = (f'<rect x="{RX + 154}" y="{y - 13}" width="128" height="18" rx="9" fill="#DCFCE7"/>'
                 f'<text x="{RX + 218}" y="{y}" text-anchor="middle" font-size="11.5" font-weight="600" fill="#166534">recognized by voice</text>')
    thursday += f'<g class="b{i}">{row(RX, i, t, who, said)}{badge}</g>'

toast = (f'<g class="toast"><g filter="url(#shadow)"><rect x="{LX + 40}" y="372" width="340" height="38" rx="19" '
         f'fill="#17171C" fill-opacity=".94"/></g>'
         f'<g transform="translate({LX + 58} 381)" fill="none" stroke="#52CF66" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
         f'<rect x="1" y="8" width="14" height="11" rx="2"/><path d="M4 8V5a4 4 0 0 1 8 0v3"/></g>'
         f'<text x="{LX + 82}" y="396" font-size="14" font-weight="600" fill="#F4F4F5">Priya’s voice kept, on this computer</text></g>')

css += f"""
@keyframes twinkle{{0%,100%{{opacity:.9}}50%{{opacity:.35}}}}
.star{{animation:twinkle 3s ease-in-out infinite}}
.star:nth-of-type(2n){{animation-delay:-1.4s}}.star:nth-of-type(3n){{animation-delay:-2.1s}}

/* Without motion: Monday named, Thursday recognized. */
@media (prefers-reduced-motion:reduce){{
  *{{animation:none!important}}
  .old,.menu,.toast,.pointer,.say-a{{opacity:0}}
  .new,.thursday,.b0,.b1,.b2,.b3,.say-b{{opacity:1;transform:none}}
}}
"""

SAY = f'x="{W / 2}" y="440" text-anchor="middle" font-size="19" font-style="italic" fill="#E6ECFF"'
STARS = "".join(
    f'<circle class="star" cx="{x}" cy="{y}" r="{r}" fill="#fff"/>'
    for x, y, r in [(480, 30, 2), (476, 210, 1.8), (930, 380, 2.2), (20, 400, 1.8), (470, 390, 1.6)])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-labelledby="t d">
<title id="t">Remembering people by voice</title>
<desc id="d">Monday's meeting note shows Speaker 1, You and Speaker 2. Clicking Speaker 2 offers the people invited; choosing Priya Shah renames every turn, and her voice is kept on this computer. In Thursday's meeting, Priya Shah's turns are labelled with her name and "recognized by voice", while a new voice stays Speaker 2.</desc>
<style>{css}</style>
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="0" y2="{H}" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0A1440"/><stop offset=".65" stop-color="#16337A"/><stop offset="1" stop-color="#2F64B8"/>
  </linearGradient>
  <linearGradient id="sea" x1="0" y1="440" x2="0" y2="{H}" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0F6FA8"/><stop offset="1" stop-color="#06305C"/>
  </linearGradient>
  <filter id="shadow" x="-20%" y="-30%" width="140%" height="180%">
    <feDropShadow dx="0" dy="8" stdDeviation="12" flood-color="#000" flood-opacity=".35"/>
  </filter>
  <filter id="soft" x="-20%" y="-30%" width="140%" height="180%">
    <feDropShadow dx="0" dy="4" stdDeviation="6" flood-color="#000" flood-opacity=".18"/>
  </filter>
  <clipPath id="card"><rect width="{W}" height="{H}" rx="28"/></clipPath>
</defs>
<g clip-path="url(#card)" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif">
  <rect width="{W}" height="{H}" fill="url(#sky)"/>
  {STARS}
  <path d="M0 458 C160 446 320 470 480 458 S800 446 960 458 V480 H0 Z" fill="url(#sea)"/>

  <!-- Monday: naming Speaker 2. -->
  <g>{monday}</g>
  {toast}

  <!-- Thursday: known by voice. -->
  <g class="thursday">{thursday}</g>

  {pointer}

  <text class="say-a" {SAY}>Name a speaker once…</text>
  <text class="say-b" {SAY}>…and they’re known by their voice in the next meeting</text>
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
