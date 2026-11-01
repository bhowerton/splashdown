# Generates docs/assets/sources.svg, the animation on the website's Notes
# section: sound from a call, a podcast app, a video in the browser, the
# room and a file each becoming a note as it plays. One 20 s loop of five
# 4 s scenes; times are percentages of it.
#
#   python3 scripts/assets/make_sources.py docs/assets/sources.svg
#   python3 scripts/assets/make_sources.py docs/assets/sources-tall.svg --tall
#
# The tall one is for phones: one source at a time, above the note.
import sys
from html import escape

out = sys.argv[1]
TALL = "--tall" in sys.argv
LOOP = 20  # seconds
F = 0.6  # fade length, %
SCENE = 20.0  # % per scene

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


# ------------------------------------------------------------------ scenes

ICONS = {
    # 24×24 stroke icons, drawn white on the tile.
    "call": '<rect x="2" y="6" width="14" height="12" rx="2"/><path d="m16 10 6-3v10l-6-3"/>',
    "podcast": '<path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="2" y="14" width="5" height="7" rx="2"/><rect x="17" y="14" width="5" height="7" rx="2"/>',
    "browser": '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 9h20"/><path d="m10 12 5 2.5-5 2.5z"/>',
    "room": '<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v4"/>',
    "file": '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/><path d="M9 14v3M12 12v5M15 14v3"/>',
}

SCENES = [
    {
        "icon": "call", "color": "#3B82F6",
        "source": "A video call", "examples": "Zoom, Teams, Meet",
        "title": "Weekly product sync", "meta": "Meeting · 34 min · You and Them",
        "rows": [("0:04", "Them", "Can we cut the pricing table?"),
                 ("0:21", "You", "Yes, three rows is plenty."),
                 ("0:48", "Them", "I'll model the annual plan.")],
        "box": ("Action items", ["☐  Model the annual plan · Them"]),
        "tags": ["pricing", "meetings"],
        "say": "A call becomes a meeting note, with its action items",
    },
    {
        "icon": "podcast", "color": "#8B5CF6",
        "source": "A podcast app", "examples": "Spotify, Podcasts",
        "title": "Writing for the ear", "meta": "Transcribed from Spotify · 42 min",
        "rows": [("0:00", "", "Welcome back to the show."),
                 ("1:12", "", "Short sentences are easier to hear."),
                 ("3:40", "", "Read it aloud before you publish.")],
        "box": ("Summary", ["•  Write the way you'd say it"]),
        "tags": ["writing"],
        "say": "A podcast becomes a transcript you can search",
    },
    {
        "icon": "browser", "color": "#EF4444",
        "source": "A video in your browser", "examples": "YouTube, webinars",
        "title": "Keynote: AI on the device", "meta": "Transcribed from Chrome · 18 min",
        "rows": [("0:00", "", "Thanks for joining us today."),
                 ("2:15", "", "Everything runs on the device."),
                 ("6:02", "", "It's available this fall.")],
        "box": ("Summary", ["•  Runs locally, ships this fall"]),
        "tags": ["research"],
        "say": "A video in your browser, written down as it plays",
    },
    {
        "icon": "room", "color": "#10B981",
        "source": "The room", "examples": "Lectures, talks",
        "title": "Biology, lecture 7", "meta": "Recorded from the microphone",
        "rows": [("0:00", "", "Today: how cells divide."),
                 ("2:40", "", "Mitosis has four phases."),
                 ("9:15", "", "This part is on the exam.")],
        "box": ("To do", ["☐  Review the four phases"]),
        "tags": ["school/biology"],
        "say": "A lecture in the room, from the microphone",
    },
    {
        "icon": "file", "color": "#F59E0B",
        "source": "A file or a link", "examples": "Voice memos, links",
        "title": "Interview with Dana", "meta": "Transcribed from interview-03.m4a",
        "rows": [("0:00", "", "Thanks for making the time."),
                 ("4:30", "", "Onboarding took too long."),
                 ("7:10", "", "The videos could be shorter.")],
        "box": ("Summary", ["•  Shorter onboarding videos"]),
        "tags": ["research/interviews"],
        "say": "A voice memo or a link, dropped onto Notes",
    },
]

# Layout.
if TALL:
    W, H = 520, 740
    SX, SW, SH = 40, 440, 66            # one source at a time
    SY = [40] * 5
    PW, PH = 206, 48
    PX, PY = (W - PW) // 2, 146          # the pill
    NX, NY, NW, NH = 40, 236, 440, 364  # the note window
    SAY_Y, SAY_SIZE = 650, 16
else:
    W, H = 960, 500
    SX, SW, SH = 40, 256, 62            # source cards
    SY = [44 + i * 76 for i in range(5)]
    PX, PY, PW, PH = 372, 206, 206, 48  # the pill
    NX, NY, NW, NH = 612, 44, 308, 364  # the note window
    SAY_Y, SAY_SIZE = 458, 18
SEA = H - 22
INK, MUTED, BLUE = "#111827", "#6B7280", "#2563EB"

sources = []
paths = []
notes = []
says = []
for i, s in enumerate(SCENES):
    start, end = i * SCENE, (i + 1) * SCENE
    k = f"k{i}"
    vis(f"on{i}", start + 0.5, end - 0.8)
    vis(f"note{i}", start + 1.6, end - 0.8)
    for j in range(len(s["rows"])):
        vis(f"row{i}{j}", start + 3.2 + j * 3.4, end - 0.8, rise=True)
    vis(f"box{i}", start + 13.4, end - 0.8, rise=True)
    vis(f"tags{i}", start + 15.6, end - 0.8, rise=True)

    y = SY[i]
    cy = y + SH / 2
    # The source, and its highlight while it plays.
    sources.append(
        f'<g class="{f"on{i} {k}" if TALL else ""}"><rect x="{SX}" y="{y}" width="{SW}" height="{SH}" rx="14" fill="#fff" fill-opacity=".96"/>'
        f'<rect x="{SX + 14}" y="{y + 13}" width="36" height="36" rx="10" fill="{s["color"]}"/>'
        f'<g transform="translate({SX + 20} {y + 19})" fill="none" stroke="#fff" stroke-width="2" '
        f'stroke-linecap="round" stroke-linejoin="round">{ICONS[s["icon"]]}</g>'
        f'<text x="{SX + 62}" y="{y + 27}" font-size="15.5" font-weight="600" fill="{INK}">{escape(s["source"])}</text>'
        f'<text x="{SX + 62}" y="{y + 46}" font-size="12.5" fill="{MUTED}">{escape(s["examples"])}</text>'
        f'<g class="on{i} {k}"><rect x="{SX - 2}" y="{y - 2}" width="{SW + 4}" height="{SH + 4}" rx="16" '
        f'fill="none" stroke="#F2551F" stroke-width="3"/>'
        + "".join(
            f'<rect class="bar" x="{SX + SW - 40 + b * 7}" y="{y + 15}" width="4" height="14" rx="2" fill="{s["color"]}"/>'
            for b in range(4))
        + "</g></g>")
    # The sound on its way to the pill.
    if TALL:
        d = f"M{W / 2} {SY[0] + SH} V{PY}"
    else:
        d = f"M{SX + SW} {cy} C{SX + SW + 50} {cy} {PX - 50} {PY + PH / 2} {PX} {PY + PH / 2}"
    paths.append(
        f'<path d="{d}" fill="none" stroke="#fff" stroke-opacity=".14" stroke-width="2"/>'
        f'<path class="on{i} {k} flow" d="{d}" fill="none" stroke="#FF9A3D" stroke-width="3" '
        f'stroke-linecap="round" stroke-dasharray="2 12"/>')

    # The note it becomes.
    x0 = NX + 20
    rows = []
    for j, (t, who, text) in enumerate(s["rows"]):
        ry = NY + 112 + j * 40
        who_span = f'<tspan font-weight="700" fill="{"#B45309" if who == "Them" else BLUE}">{who} </tspan>' if who else ""
        rows.append(
            f'<g class="row{i}{j} {k}"><text x="{x0}" y="{ry}" font-size="12" fill="{BLUE}" '
            f'font-family="ui-monospace, SFMono-Regular, Menlo, monospace">{t}</text>'
            f'<text x="{x0 + 42}" y="{ry}" font-size="14" fill="{INK}">{who_span}{escape(text)}</text></g>')
    head, lines = s["box"]
    by = NY + 232
    box = (f'<g class="box{i} {k}"><rect x="{x0 - 6}" y="{by}" width="{NW - 28}" height="{30 + 20 * len(lines)}" '
           f'rx="10" fill="#EEF4FF"/><text x="{x0 + 6}" y="{by + 21}" font-size="12" font-weight="700" '
           f'fill="{BLUE}" letter-spacing=".04em">{escape(head.upper())}</text>'
           + "".join(f'<text x="{x0 + 6}" y="{by + 42 + 20 * n}" font-size="13.5" fill="{INK}">{escape(l)}</text>'
                     for n, l in enumerate(lines))
           + "</g>")
    tx = x0
    chips = []
    for t in s["tags"]:
        w = 16 + 7.4 * (len(t) + 1)
        chips.append(f'<rect x="{tx}" y="{NY + 318}" width="{w:.0f}" height="24" rx="12" fill="#E0E7FF"/>'
                     f'<text x="{tx + w / 2:.0f}" y="{NY + 334}" text-anchor="middle" font-size="12.5" fill="#3730A3">#{escape(t)}</text>')
        tx += w + 8
    notes.append(
        f'<g class="note{i} {k}">'
        f'<text x="{x0}" y="{NY + 70}" font-size="18" font-weight="700" fill="{INK}">{escape(s["title"])}</text>'
        f'<text x="{x0}" y="{NY + 90}" font-size="12.5" fill="{MUTED}">{escape(s["meta"])}</text>'
        + "".join(rows) + box + f'<g class="tags{i} {k}">{"".join(chips)}</g></g>')
    vis(f"say{i}", start + 0.5, end - 0.8)
    says.append(f'<text class="say{i} {k}" x="{W / 2}" y="{SAY_Y}" text-anchor="middle" font-size="{SAY_SIZE}" '
                f'font-style="italic" fill="#E6ECFF">{escape(s["say"])}</text>')

css += f"""
/* Sound flowing into the pill, and its level. */
@keyframes flow{{to{{stroke-dashoffset:-28}}}}
.flow{{animation-name:on0,flow;animation-duration:{LOOP}s,.6s;animation-timing-function:linear;animation-iteration-count:infinite}}
.flow.on1{{animation-name:on1,flow}}.flow.on2{{animation-name:on2,flow}}.flow.on3{{animation-name:on3,flow}}.flow.on4{{animation-name:on4,flow}}
@keyframes bar{{0%{{transform:scaleY(.25)}}100%{{transform:scaleY(1)}}}}
.bar{{transform-box:fill-box;transform-origin:center;animation:bar .45s ease-in-out infinite alternate}}
.bar:nth-of-type(2n){{animation-delay:-.15s}}.bar:nth-of-type(3n){{animation-delay:-.3s}}
@keyframes twinkle{{0%,100%{{opacity:.9}}50%{{opacity:.35}}}}
.star{{animation:twinkle 3s ease-in-out infinite}}
.star:nth-of-type(2n){{animation-delay:-1.4s}}.star:nth-of-type(3n){{animation-delay:-2.1s}}

/* Without motion: the call, as a finished note. */
@media (prefers-reduced-motion:reduce){{
  *{{animation:none!important}}
  .k1,.k2,.k3,.k4{{opacity:0}}
  .k0{{opacity:1;transform:none}}
}}
"""

wave = "".join(
    f'<rect class="bar" x="{PX + 148 + b * 8}" y="{PY + 15}" width="4" height="18" rx="2" fill="#FF9A3D"/>'
    for b in range(6))
pill = (f'<g filter="url(#shadow)"><rect x="{PX}" y="{PY}" width="{PW}" height="{PH}" rx="24" '
        f'fill="#17171C" fill-opacity=".94"/></g>'
        f'<circle cx="{PX + 24}" cy="{PY + 24}" r="6" fill="#F2551F"/>'
        f'<text x="{PX + 38}" y="{PY + 30}" font-size="16" font-weight="600" fill="#F4F4F5">Transcribing</text>'
        f'<g>{wave}</g>')
# From the pill into the note.
into = (f'<path d="{f"M{W / 2} {PY + PH} V{NY}" if TALL else f"M{PX + PW} {PY + PH / 2} H{NX}"}" fill="none" stroke="#FF9A3D" stroke-width="3" '
        f'stroke-linecap="round" stroke-dasharray="2 12" class="pulse"/>')
css += ".pulse{animation:flow .6s linear infinite}\n"

STARS = "".join(
    f'<circle class="star" cx="{x * W:.0f}" cy="{y * H:.0f}" r="{r}" fill="#fff"/>'
    for x, y, r in [(.34, .08, 2.2), (.54, .22, 1.8), (.98, .4, 2.4), (.36, .76, 2), (.58, .66, 1.6), (.02, .5, 1.8)])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-labelledby="t d">
<title id="t">From any sound to a note</title>
<desc id="d">Five sources take turns: a video call, a podcast app, a video in the browser, the room and a file. As each plays, its sound flows into Splashdown and becomes a note line by line, with time stamps, a summary or action items, and tags: the call becomes a meeting note with You and Them, the podcast and the video become transcripts, the lecture is recorded from the microphone, and an interview file is transcribed.</desc>
<style>{css}</style>
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="0" y2="{H}" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0A1440"/><stop offset=".65" stop-color="#16337A"/><stop offset="1" stop-color="#2F64B8"/>
  </linearGradient>
  <linearGradient id="sea" x1="0" y1="{SEA - 8}" x2="0" y2="{H}" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0F6FA8"/><stop offset="1" stop-color="#06305C"/>
  </linearGradient>
  <filter id="shadow" x="-20%" y="-30%" width="140%" height="180%">
    <feDropShadow dx="0" dy="8" stdDeviation="12" flood-color="#000" flood-opacity=".35"/>
  </filter>
  <clipPath id="card"><rect width="{W}" height="{H}" rx="28"/></clipPath>
  <clipPath id="window"><rect x="{NX}" y="{NY}" width="{NW}" height="{NH}" rx="14"/></clipPath>
</defs>
<g clip-path="url(#card)" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif">
  <rect width="{W}" height="{H}" fill="url(#sky)"/>
  {STARS}
  <path d="M0 {SEA} C{W / 6} {SEA - 12} {W / 3} {SEA + 12} {W / 2} {SEA} S{W * 5 / 6} {SEA - 12} {W} {SEA} V{H} H0 Z" fill="url(#sea)"/>

  <!-- Where the sound comes from. -->
  {"".join(paths)}
  {"".join(sources)}

  <!-- Splashdown, transcribing. -->
  {pill}
  {into}

  <!-- The note it becomes. -->
  <g filter="url(#shadow)"><rect x="{NX}" y="{NY}" width="{NW}" height="{NH}" rx="14" fill="#fff"/></g>
  <g clip-path="url(#window)">
    <rect x="{NX}" y="{NY}" width="{NW}" height="36" fill="#EEF1F6"/>
    <circle cx="{NX + 20}" cy="{NY + 18}" r="5.5" fill="#FF5F57"/><circle cx="{NX + 38}" cy="{NY + 18}" r="5.5" fill="#FEBC2E"/><circle cx="{NX + 56}" cy="{NY + 18}" r="5.5" fill="#28C840"/>
    <text x="{NX + NW / 2}" y="{NY + 23}" text-anchor="middle" font-size="13" fill="{MUTED}">Notes</text>
    {"".join(notes)}
  </g>

  <!-- What's happening. -->
  {"".join(says)}
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
