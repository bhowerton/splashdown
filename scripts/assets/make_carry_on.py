# Generates docs/assets/carry-on.svg, the animation for carrying on a
# finished meeting: Carry on recording in the note's menu, new turns timed
# from where the note stopped, then the summary and the recording covering
# all of it. One 18 s loop of three parts; times are percentages of it.
#
#   python3 scripts/assets/make_carry_on.py docs/assets/carry-on.svg
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


INK, MUTED, BLUE = "#111827", "#6B7280", "#2563EB"
ORANGE, AMBER = "#F2551F", "#FF9A3D"
DARK, LIGHT = "#17171C", "#F4F4F5"
MONO = 'font-family="ui-monospace, SFMono-Regular, Menlo, monospace"'
W, H = 960, 480

# Layout: the meeting's note on the left; on the right its menu, then the
# recording pill, above its summary.
NX, NY, NW, NH = 40, 36, 560, 380
MX, MY, MW = 660, 44, 232
PX, PY, PW = 666, 84, 206
SX, SY, SW, SH = 632, 212, 288, 204


def chrome(x, y, w, title):
    return (f'<rect x="{x}" y="{y}" width="{w}" height="34" rx="14" fill="#EEF1F6"/>'
            f'<rect x="{x}" y="{y + 20}" width="{w}" height="14" fill="#EEF1F6"/>'
            f'<circle cx="{x + 20}" cy="{y + 17}" r="5.5" fill="#FF5F57"/><circle cx="{x + 38}" cy="{y + 17}" r="5.5" fill="#FEBC2E"/>'
            f'<circle cx="{x + 56}" cy="{y + 17}" r="5.5" fill="#28C840"/>'
            f'<text x="{x + w / 2 + 20}" y="{y + 22}" text-anchor="middle" font-size="13" fill="{MUTED}">{escape(title)}</text>')


def window(x, y, w, h, title):
    return (f'<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="14" fill="#fff"/></g>'
            + chrome(x, y, w, title))


def turn(y, stamp, who, text):
    color = BLUE if who == "You" else "#B45309"
    return (f'<text x="{NX + 22}" y="{y}" font-size="12" fill="{BLUE}" {MONO}>{stamp}</text>'
            f'<text x="{NX + 76}" y="{y}" font-size="14" fill="{INK}"><tspan font-weight="700" fill="{color}">{who}  </tspan>{escape(text)}</text>')


def aside(y, text):
    return f'<text x="{NX + 22}" y="{y}" font-size="13" font-style="italic" fill="{MUTED}">{escape(text)}</text>'


# ------------------------------------------------------------------ timing
vis("menu", 5, 29, rise=True)
vis("pick", 15, 29)
vis("recording", 31, 68, rise=True)
vis("carried", 33, 99.4, rise=True)
vis("new0", 41, 99.4, rise=True)
vis("new1", 51, 99.4, rise=True)
vis("ended", 69, 99.4, rise=True)
vis("short", 0.6, 68)
vis("long", 69, 99.4)
vis("again", 76, 99.4, rise=True)
vis("say0", 1, 29)
vis("say1", 31, 68)
vis("say2", 69, 99)

# ---------------------------------------------------------------- the note
TY = NY + 126
# The recording under the transcript: 12:40 long, then 21:15, the new part
# joined on where the first one ended.
BX, BY, BW = NX + 56, NY + NH - 34, NW - 150
JOIN = BW * 0.6
note = (window(NX, NY, NW, NH, "Notes")
        + f'<text x="{NX + 22}" y="{NY + 68}" font-size="18" font-weight="700" fill="{INK}">Design review</text>'
        + f'<text x="{NX + 22}" y="{NY + 90}" font-size="12.5" fill="{MUTED}">Thursday 10:00 · with Priya Shah, Dana Ortiz</text>'
        + turn(TY, "12:18", "Them", "So we ship the beta on the 17th.")
        + turn(TY + 30, "12:31", "You", "Agreed. I'll send Dana the numbers.")
        + aside(TY + 60, "Ended at 12:40.")
        + f'<g class="carried">{aside(TY + 96, "Carried on at 12:40, Nov 12, 10:52 AM.")}'
        + f'<rect x="{NX + 12}" y="{TY + 80}" width="3" height="22" rx="1.5" fill="{ORANGE}"/></g>'
        + f'<g class="new0">{turn(TY + 130, "12:46", "Them", "One more thing: the pricing page.")}</g>'
        + f'<g class="new1">{turn(TY + 160, "13:05", "You", "I can review it before Friday.")}</g>'
        + f'<g class="ended">{aside(TY + 190, "Ended at 21:15.")}</g>'
        + f'<path d="M{NX + 24} {BY - 7} l11 7 -11 7z" fill="{INK}"/>'
        + f'<rect x="{BX}" y="{BY - 2}" width="{BW}" height="5" rx="2.5" fill="#E5E7EB"/>'
        + f'<g class="short"><rect x="{BX}" y="{BY - 2}" width="{BW}" height="5" rx="2.5" fill="#9CA3AF"/>'
        + f'<text x="{NX + NW - 22}" y="{BY + 5}" text-anchor="end" font-size="12.5" fill="{MUTED}" {MONO}>12:40</text></g>'
        + f'<g class="long"><rect x="{BX}" y="{BY - 2}" width="{JOIN:.0f}" height="5" rx="2.5" fill="#9CA3AF"/>'
        + f'<rect x="{BX + JOIN + 3:.0f}" y="{BY - 2}" width="{BW - JOIN - 3:.0f}" height="5" rx="2.5" fill="{ORANGE}"/>'
        + f'<text x="{NX + NW - 22}" y="{BY + 5}" text-anchor="end" font-size="12.5" fill="{MUTED}" {MONO}>21:15</text></g>')

# ---------------------------------------------------- its menu and the pill
items = ["Rename…", "Move to…", "Carry on recording", "Transcribe again…"]
menu = (f'<g filter="url(#shadow)"><rect x="{MX}" y="{MY}" width="{MW}" height="{len(items) * 32 + 16}" rx="12" fill="#fff"/></g>'
        + f'<rect class="pick" x="{MX + 6}" y="{MY + 8 + 2 * 32}" width="{MW - 12}" height="32" rx="7" fill="{BLUE}"/>')
for i, item in enumerate(items):
    y = MY + 29 + i * 32
    menu += f'<text x="{MX + 18}" y="{y}" font-size="14" fill="{INK}">{escape(item)}</text>'
    if i == 2:
        menu += f'<text class="pick" x="{MX + 18}" y="{y}" font-size="14" fill="#fff">{escape(item)}</text>'

wave = "".join(
    f'<rect class="bar" x="{PX + PW - 58 + b * 8}" y="{PY + 15}" width="4" height="18" rx="2" fill="{AMBER}"/>'
    for b in range(6))
pill = (f'<g filter="url(#shadow)"><rect x="{PX}" y="{PY}" width="{PW}" height="48" rx="24" fill="{DARK}" fill-opacity=".94"/></g>'
        f'<circle cx="{PX + 24}" cy="{PY + 24}" r="6" fill="{ORANGE}"/>'
        f'<text x="{PX + 38}" y="{PY + 30}" font-size="16" font-weight="600" fill="{LIGHT}">Recording</text>'
        f'<g>{wave}</g>')

# ------------------------------------------------------------- the summary
summary = (window(SX, SY, SW, SH, "Summary")
           + f'<text x="{SX + 20}" y="{SY + 62}" font-size="11.5" font-weight="700" fill="{MUTED}" letter-spacing=".05em">ACTION ITEMS</text>'
           + f'<text x="{SX + 20}" y="{SY + 90}" font-size="14" fill="{INK}">• Ship the beta on the 17th</text>'
           + f'<text x="{SX + 20}" y="{SY + 116}" font-size="14" fill="{INK}">• You: send Dana the numbers</text>'
           + f'<g class="again"><text x="{SX + 20}" y="{SY + 142}" font-size="14" fill="{INK}">• You: review the pricing page</text>'
           + f'<rect x="{SX + 20}" y="{SY + 158}" width="186" height="24" rx="12" fill="#FFEDD5"/>'
           + f'<text x="{SX + 113}" y="{SY + 174.5}" text-anchor="middle" font-size="12" font-weight="600" fill="#C2410C">Written again, from all of it</text></g>')

css += f"""
@keyframes bar{{0%{{transform:scaleY(.25)}}100%{{transform:scaleY(1)}}}}
.bar{{transform-box:fill-box;transform-origin:center;animation:bar .45s ease-in-out infinite alternate}}
.bar:nth-of-type(2n){{animation-delay:-.15s}}.bar:nth-of-type(3n){{animation-delay:-.3s}}
@keyframes twinkle{{0%,100%{{opacity:.9}}50%{{opacity:.35}}}}
.star{{animation:twinkle 3s ease-in-out infinite}}
.star:nth-of-type(2n){{animation-delay:-1.4s}}.star:nth-of-type(3n){{animation-delay:-2.1s}}

/* Without motion: the note carried on, its summary and recording whole. */
@media (prefers-reduced-motion:reduce){{
  *{{animation:none!important}}
  .menu,.pick,.recording,.short,.say0,.say1{{opacity:0}}
  .carried,.new0,.new1,.ended,.long,.again,.say2{{opacity:1;transform:none}}
}}
"""

SAY = f'x="{W / 2}" y="448" text-anchor="middle" font-size="19" font-style="italic" fill="#E6ECFF"'
STARS = "".join(
    f'<circle class="star" cx="{x}" cy="{y}" r="{r}" fill="#fff"/>'
    for x, y, r in [(620, 30, 2), (940, 60, 1.8), (20, 440, 1.8), (616, 190, 1.6), (944, 430, 2)])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-labelledby="t d">
<title id="t">Carrying on a meeting in the same note</title>
<desc id="d">A meeting note, Design review, ends at 12:40. Carry on recording is chosen from its menu. A line in the note marks where it was carried on, and the new turns are timed from there: 12:46, 13:05. When it ends at 21:15, the summary is written again from the whole meeting and gains an action item, and the recording under the transcript is one recording, 21:15 long.</desc>
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
  <clipPath id="card"><rect width="{W}" height="{H}" rx="28"/></clipPath>
</defs>
<g clip-path="url(#card)" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif">
  <rect width="{W}" height="{H}" fill="url(#sky)"/>
  {STARS}
  <path d="M0 462 C160 450 320 474 480 462 S800 450 960 462 V480 H0 Z" fill="url(#sea)"/>

  <!-- The meeting's note, with its recording under the transcript. -->
  {note}

  <!-- Its summary. -->
  {summary}

  <!-- The note's menu, then Splashdown recording. -->
  <g class="menu">{menu}</g>
  <g class="recording">{pill}</g>

  <text class="say0" {SAY}>A meeting stopped too soon, or picked up again after a break…</text>
  <text class="say1" {SAY}>…carry on in the same note, timed from where it stopped…</text>
  <text class="say2" {SAY}>…and the summary and the recording cover all of it.</text>
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
