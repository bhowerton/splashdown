# Generates docs/assets/hooks.svg, the animation for hooks and the offer to
# join a meeting: one meeting from before it starts to the end of the week.
# A brief ten minutes before, a card to join and record as it starts,
# reminders and a draft follow-up when it ends, and a weekly review on
# Friday. One 26 s loop of four scenes; times are percentages of it.
#
#   python3 scripts/assets/make_hooks.py docs/assets/hooks.svg
import sys
from html import escape

out = sys.argv[1]
LOOP = 26
F = 0.5
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
DARK, LIGHT, DIM = "#17171C", "#F4F4F5", "#A1A1AA"
W, H = 960, 480

# The four scenes, and when each one's parts come in.
SCENES = [(1, 24), (26, 49), (51, 74), (76, 99)]
for i, (a, b) in enumerate(SCENES):
    vis(f"s{i}", a, b)
    vis(f"t{i}", a, b)
    vis(f"say{i}", a, b)


def chrome(x, y, w, title):
    return (f'<rect x="{x}" y="{y}" width="{w}" height="34" rx="14" fill="#EEF1F6"/>'
            f'<rect x="{x}" y="{y + 20}" width="{w}" height="14" fill="#EEF1F6"/>'
            f'<circle cx="{x + 20}" cy="{y + 17}" r="5.5" fill="#FF5F57"/><circle cx="{x + 38}" cy="{y + 17}" r="5.5" fill="#FEBC2E"/>'
            f'<circle cx="{x + 56}" cy="{y + 17}" r="5.5" fill="#28C840"/>'
            f'<text x="{x + w / 2 + 20}" y="{y + 22}" text-anchor="middle" font-size="13" fill="{MUTED}">{escape(title)}</text>')


def window(x, y, w, h, title):
    return (f'<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="14" fill="#fff"/></g>'
            + chrome(x, y, w, title))


def panel(x, y, w, h):
    """One of Splashdown's own small panels: dark, with its name on it."""
    return (f'<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="18" fill="{DARK}" fill-opacity=".96"/></g>'
            f'<circle cx="{x + 24}" cy="{y + 24}" r="5" fill="{ORANGE}"/>'
            f'<text x="{x + 36}" y="{y + 28.5}" font-size="12.5" font-weight="600" fill="{DIM}">Splashdown</text>')


def button(x, y, w, label, primary=False):
    fill = f'fill="url(#go)"' if primary else 'fill="#fff" fill-opacity=".1"'
    return (f'<rect x="{x}" y="{y}" width="{w}" height="30" rx="15" {fill}/>'
            f'<text x="{x + w / 2}" y="{y + 20}" text-anchor="middle" font-size="13.5" font-weight="600" fill="#fff">{escape(label)}</text>')


def pill(x, y, label, w=206):
    wave = "".join(
        f'<rect class="bar" x="{x + w - 58 + b * 8}" y="{y + 15}" width="4" height="18" rx="2" fill="{AMBER}"/>'
        for b in range(6))
    return (f'<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="{w}" height="48" rx="24" fill="{DARK}" fill-opacity=".94"/></g>'
            f'<circle cx="{x + 24}" cy="{y + 24}" r="6" fill="{ORANGE}"/>'
            f'<text x="{x + 38}" y="{y + 30}" font-size="16" font-weight="600" fill="{LIGHT}">{escape(label)}</text>'
            f'<g>{wave}</g>')


def flow(d):
    return (f'<path class="flow" d="{d}" fill="none" stroke="{AMBER}" stroke-width="3" '
            f'stroke-linecap="round" stroke-dasharray="2 12"/>')


def heading(x, y, text):
    return (f'<text x="{x}" y="{y}" font-size="11.5" font-weight="700" fill="{MUTED}" '
            f'letter-spacing=".05em">{escape(text.upper())}</text>')


def line(x, y, text, size=14, fill=INK, weight=""):
    w = f' font-weight="{weight}"' if weight else ""
    return f'<text x="{x}" y="{y}" font-size="{size}" fill="{fill}"{w}>{escape(text)}</text>'


def todo(x, y, text):
    return (f'<rect x="{x}" y="{y - 12}" width="14" height="14" rx="4" fill="none" stroke="#9CA3AF" stroke-width="1.5"/>'
            + line(x + 22, y, text))


# ---------------------------------------------------------------- the rail
# When each scene happens, along the top.
STOPS = [(150, "Thu 9:50"), (370, "9:58"), (590, "10:45"), (810, "Fri 4:00 PM")]
rail = f'<path d="M{STOPS[0][0]} 52 H{STOPS[-1][0]}" stroke="#fff" stroke-opacity=".22" stroke-width="2"/>'
for i, (x, label) in enumerate(STOPS):
    rail += (f'<circle cx="{x}" cy="52" r="5" fill="#8FA3D9"/>'
             f'<text x="{x}" y="36" text-anchor="middle" font-size="13" fill="#B9C4E8">{label}</text>'
             f'<g class="t{i}"><circle cx="{x}" cy="52" r="8" fill="{ORANGE}" stroke="#fff" stroke-width="2"/>'
             f'<text x="{x}" y="36" text-anchor="middle" font-size="13" fill="#fff">{label}</text></g>')

# ------------------------------------------------- 1: a brief, before it
CX, CY, CW, CH = 60, 96, 300, 236
BX, BY, BW, BH = 470, 104, 430, 220


def event(y, time, title, now=False):
    if not now:
        return line(CX + 20, y, time, 13, MUTED) + line(CX + 74, y, title, 14, MUTED)
    return (f'<rect x="{CX + 12}" y="{y - 22}" width="{CW - 24}" height="34" rx="8" fill="#EEF4FF"/>'
            f'<rect x="{CX + 12}" y="{y - 22}" width="4" height="34" rx="2" fill="{BLUE}"/>'
            + line(CX + 24, y, time, 13, BLUE, "700") + line(CX + 74, y, title, 14, INK, "700")
            + f'<rect x="{CX + CW - 96}" y="{y - 15}" width="76" height="20" rx="10" fill="#FFEDD5"/>'
            + f'<text x="{CX + CW - 58}" y="{y - 1}" text-anchor="middle" font-size="11.5" font-weight="600" fill="#C2410C">in 10 min</text>')


brief = ["Last time, you decided to ship the beta on the 17th.",
         "You said you'd send Dana the numbers.",
         "Dana owes you the new mockups."]
for i in range(len(brief)):
    vis(f"b{i}", 7 + i * 3.5, 24, rise=True)
vis("brief", 5, 24, rise=True)
scene0 = (window(CX, CY, CW, CH, "Calendar")
          + heading(CX + 20, CY + 62, "Thursday")
          + event(CY + 96, "9:00", "Standup")
          + event(CY + 138, "10:00", "Design review", now=True)
          + event(CY + 180, "13:00", "Lunch with Sam")
          + event(CY + 214, "15:30", "1:1 with Priya")
          + flow(f"M{CX + CW} {CY + 132} C{CX + CW + 50} {CY + 132} {BX - 50} {BY + 60} {BX} {BY + 60}")
          + f'<g class="brief">{panel(BX, BY, BW, BH)}'
          + line(BX + 22, BY + 64, "Brief: Design review", 18, "#fff", "700")
          + "".join(f'<g class="b{i}">{line(BX + 22, BY + 98 + i * 28, t, 14, "#D4D4D8")}</g>' for i, t in enumerate(brief))
          + button(BX + BW - 98, BY + BH - 46, 76, "Open", primary=True) + "</g>")

# -------------------------------------- 2: join and record, as it starts
OX, OY, OW, OH = 240, 96, 480, 212
# Join is the button to press first; once it's pressed, Record is.
vis("offer", 26, 37.5)
vis("join-first", 26, 32)
vis("press-join", 30, 32)
vis("record-next", 33, 37.5)
vis("press", 35.5, 37.5)
vis("recording", 38.5, 49, rise=True)
vis("named", 40, 49, rise=True)
vis("turn", 43, 49, rise=True)
RX, RY = 377, 104
MX, MY, MW, MH = 300, 176, 360, 170
scene1 = (f'<g class="offer">{panel(OX, OY, OW, OH)}'
          + line(OX + 22, OY + 66, "Your meeting starts in 2 minutes", 19, "#fff", "700")
          + line(OX + 22, OY + 92, "Design review · with Priya Shah, Dana Ortiz", 14, "#D4D4D8")
          + line(OX + 22, OY + 120, "It's on Zoom. Record it into your notes?", 14, "#D4D4D8")
          + line(OX + 22, OY + 144, "Let people know you're recording.", 12.5, DIM)
          + button(OX + OW - 276, OY + OH - 46, 86, "Not now")
          + f'<g class="join-first">{button(OX + OW - 182, OY + OH - 46, 64, "Join", primary=True)}'
          + button(OX + OW - 110, OY + OH - 46, 88, "Record") + "</g>"
          + f'<rect class="press-join" x="{OX + OW - 186}" y="{OY + OH - 50}" width="72" height="38" rx="19" fill="none" stroke="#fff" stroke-width="2"/>'
          + f'<g class="record-next">{button(OX + OW - 182, OY + OH - 46, 64, "Join")}'
          + button(OX + OW - 110, OY + OH - 46, 88, "Record", primary=True) + "</g>"
          + f'<rect class="press" x="{OX + OW - 114}" y="{OY + OH - 50}" width="96" height="38" rx="19" fill="none" stroke="#fff" stroke-width="2"/></g>'
          + f'<g class="recording">{pill(RX, RY, "Recording")}</g>'
          + f'<g class="named">{window(MX, MY, MW, MH, "Notes")}'
          + line(MX + 20, MY + 68, "Design review", 18, INK, "700")
          + line(MX + 20, MY + 90, "With Priya Shah, Dana Ortiz", 12.5, MUTED)
          + f'<g class="turn"><text x="{MX + 20}" y="{MY + 126}" font-size="14" fill="{INK}">'
          + f'<tspan font-weight="700" fill="{BLUE}">0:02 You  </tspan>Morning, everyone.</text></g></g>')

# ------------------------- 3: reminders and a draft, when it ends
NX, NY, NW, NH = 40, 96, 290, 290
PX, PY, PW = 356, 217, 196
TX, TY, TW, TH = 580, 96, 340, 132
DX, DY, DW, DH = 580, 244, 340, 142
summary = ["Beta ships on the 17th.", "Dana sends the new mockups.", "You send Dana the numbers.", "Book the usability room."]
todos = ["Send Dana the numbers", "Book the usability room"]
vis("asking2", 55, 66)
for i in range(len(todos)):
    vis(f"todo{i}", 58 + i * 2.5, 74, rise=True)
vis("draft", 64, 74, rise=True)
scene2 = (window(NX, NY, NW, NH, "Notes")
          + line(NX + 20, NY + 68, "Design review", 18, INK, "700")
          + line(NX + 20, NY + 90, "Thursday 10:00 · 45 min", 12.5, MUTED)
          + heading(NX + 20, NY + 124, "Summary")
          + "".join(line(NX + 20, NY + 152 + i * 28, f"• {t}") for i, t in enumerate(summary))
          + f'<g class="asking2">{pill(PX, PY, "Asking the AI", PW)}'
          + flow(f"M{NX + NW} {PY + 24} H{PX}")
          + flow(f"M{PX + PW} {PY + 24} C{PX + PW + 20} {PY + 24} {TX - 20} {TY + 70} {TX} {TY + 70}")
          + flow(f"M{PX + PW} {PY + 24} C{PX + PW + 20} {PY + 24} {DX - 20} {DY + 70} {DX} {DY + 70}") + "</g>"
          + window(TX, TY, TW, TH, "Reminders")
          + "".join(f'<g class="todo{i}">{todo(TX + 20, TY + 68 + i * 32, t)}</g>' for i, t in enumerate(todos))
          + window(DX, DY, DW, DH, "New message")
          + f'<g class="draft">{line(DX + 20, DY + 64, "Design review: what we agreed", 14.5, INK, "700")}'
          + line(DX + 20, DY + 90, "Hi Dana and Priya, thanks for today. The beta", 13.5, MUTED)
          + line(DX + 20, DY + 110, "ships on the 17th, and…", 13.5, MUTED)
          + f'<rect x="{DX + DW - 150}" y="{DY + 100}" width="132" height="24" rx="12" fill="#FFEDD5"/>'
          + f'<text x="{DX + DW - 84}" y="{DY + 116.5}" text-anchor="middle" font-size="12" font-weight="600" fill="#C2410C">Draft · you send it</text></g>')

# ---------------------------------------- 4: a weekly review, on Friday
LX, LY, LW, LH = 60, 96, 280, 250
QX, QY, QW = 372, 197, 196
WX, WY, WW, WH = 600, 96, 320, 300
week = [("Mon", "Standup"), ("Tue", "1:1 with Priya"), ("Thu", "Design review"), ("All week", "Inbox, 6 lines")]
review = [("h", "Decided"), ("l", "• Beta ships on the 17th."),
          ("h", "You said you'd"), ("t", "Send Dana the numbers"), ("t", "Reply to Priya about hiring"),
          ("h", "Still open"), ("l", "• Pricing for teams")]
vis("asking3", 79, 90)
for i in range(len(review)):
    vis(f"w{i}", 82 + i * 1.6, 99, rise=True)
rows, y = "", WY + 120
for i, (kind, text) in enumerate(review):
    y += 6 if kind == "h" and i else 0
    part = heading(WX + 20, y, text) if kind == "h" else todo(WX + 20, y, text) if kind == "t" else line(WX + 20, y, text)
    rows += f'<g class="w{i}">{part}</g>'
    y += 22 if kind == "h" else 25
scene3 = (window(LX, LY, LW, LH, "Notes")
          + heading(LX + 20, LY + 62, "This week")
          + "".join(
              f'<rect x="{LX + 20}" y="{LY + 80 + i * 40}" width="16" height="20" rx="3" fill="{"#94A3B8" if i < 3 else AMBER}"/>'
              + line(LX + 46, LY + 95 + i * 40, title, 14)
              + f'<text x="{LX + LW - 20}" y="{LY + 95 + i * 40}" text-anchor="end" font-size="12.5" fill="{MUTED}">{day}</text>'
              for i, (day, title) in enumerate(week))
          + f'<g class="asking3">{pill(QX, QY, "Asking the AI", QW)}'
          + flow(f"M{LX + LW} {QY + 24} H{QX}") + flow(f"M{QX + QW} {QY + 24} H{WX}") + "</g>"
          + window(WX, WY, WW, WH, "Notes")
          + line(WX + 20, WY + 68, "Weekly review", 18, INK, "700")
          + line(WX + 20, WY + 90, "Friday 4:00 PM · from 4 notes", 12.5, MUTED)
          + rows)

css += f"""
@keyframes flow{{to{{stroke-dashoffset:-28}}}}
.flow{{animation:flow .6s linear infinite}}
@keyframes bar{{0%{{transform:scaleY(.25)}}100%{{transform:scaleY(1)}}}}
.bar{{transform-box:fill-box;transform-origin:center;animation:bar .45s ease-in-out infinite alternate}}
.bar:nth-of-type(2n){{animation-delay:-.15s}}.bar:nth-of-type(3n){{animation-delay:-.3s}}
@keyframes twinkle{{0%,100%{{opacity:.9}}50%{{opacity:.35}}}}
.star{{animation:twinkle 3s ease-in-out infinite}}
.star:nth-of-type(2n){{animation-delay:-1.4s}}.star:nth-of-type(3n){{animation-delay:-2.1s}}

/* Without motion: when the meeting ends, the reminders and the draft. */
@media (prefers-reduced-motion:reduce){{
  *{{animation:none!important}}
  .s0,.s1,.s3,.t0,.t1,.t3,.say0,.say1,.say3,.asking2{{opacity:0}}
  .s2,.t2,.say2,.todo0,.todo1,.draft{{opacity:1;transform:none}}
}}
"""

SAY = f'x="{W / 2}" y="440" text-anchor="middle" font-size="19" font-style="italic" fill="#E6ECFF"'
SAYS = ["Ten minutes before a meeting, a brief from your last notes about it…",
        "…as it starts, a card to join the call and record it…",
        "…when it ends, your to-dos are reminders and the follow-up is a draft…",
        "…and on Friday, the week's notes become a review."]
STARS = "".join(
    f'<circle class="star" cx="{x}" cy="{y}" r="{r}" fill="#fff"/>'
    for x, y, r in [(40, 60, 2), (930, 30, 1.8), (480, 76, 1.6), (20, 400, 1.8), (940, 410, 2)])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-labelledby="t d">
<title id="t">Around a meeting: a brief, a card to join and record, reminders, a draft and a weekly review</title>
<desc id="d">Thursday, 9:50: ten minutes before Design review on the calendar, a panel shows a brief from the last notes about it: what was decided, what you said you'd do and what someone owes you. At 9:58 a card says the meeting starts in 2 minutes and offers to join it on Zoom and record it; Join opens the call, and Record starts a note named from the calendar. At 10:45 the meeting ends: the AI turns its to-dos into reminders and writes a follow-up email that waits as a draft for you to send. On Friday at 4:00 PM, the week's notes become a weekly review: what was decided, what you said you'd do, and what is still open.</desc>
<style>{css}</style>
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="0" y2="{H}" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0A1440"/><stop offset=".65" stop-color="#16337A"/><stop offset="1" stop-color="#2F64B8"/>
  </linearGradient>
  <linearGradient id="sea" x1="0" y1="440" x2="0" y2="{H}" gradientUnits="userSpaceOnUse">
    <stop offset="0" stop-color="#0F6FA8"/><stop offset="1" stop-color="#06305C"/>
  </linearGradient>
  <linearGradient id="go" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{AMBER}"/><stop offset="1" stop-color="{ORANGE}"/>
  </linearGradient>
  <filter id="shadow" x="-20%" y="-30%" width="140%" height="180%">
    <feDropShadow dx="0" dy="8" stdDeviation="12" flood-color="#000" flood-opacity=".35"/>
  </filter>
  <clipPath id="card"><rect width="{W}" height="{H}" rx="28"/></clipPath>
</defs>
<g clip-path="url(#card)" font-family="-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif">
  <rect width="{W}" height="{H}" fill="url(#sky)"/>
  {STARS}
  <path d="M0 458 C160 446 320 470 480 458 S800 446 960 458 V480 H0 Z" fill="url(#sea)"/>

  <!-- When each thing happens. -->
  {rail}

  <!-- Before the meeting: a brief. -->
  <g class="s0">{scene0}</g>

  <!-- As it starts: join and record. -->
  <g class="s1">{scene1}</g>

  <!-- When it ends: reminders and a draft. -->
  <g class="s2">{scene2}</g>

  <!-- On Friday: a weekly review. -->
  <g class="s3">{scene3}</g>

  {"".join(f'<text class="say{i}" {SAY}>{escape(s)}</text>' for i, s in enumerate(SAYS))}
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
