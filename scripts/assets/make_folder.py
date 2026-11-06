# Generates docs/assets/folder.svg, the animation for watched folders and
# subtitles: a recording copied into a watched folder becomes a transcript
# note by itself, and its subtitles are saved beside it, shown under the
# video. One 18 s loop; times are percentages of it.
#
#   python3 scripts/assets/make_folder.py docs/assets/folder.svg
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
MONO = 'font-family="ui-monospace, SFMono-Regular, Menlo, monospace"'
W, H = 960, 480

# Layout: the folder top left, the video under it, the pill in the middle,
# the note on the right.
FX, FY, FW, FH = 40, 40, 300, 176
VX, VY, VW, VH = 40, 236, 300, 168
PX, PY, PW, PH = 372, 196, 206, 48
NX, NY, NW, NH = 612, 40, 308, 364


def chrome(x, y, w, title):
    return (f'<rect x="{x}" y="{y}" width="{w}" height="34" rx="14" fill="#EEF1F6"/>'
            f'<rect x="{x}" y="{y + 20}" width="{w}" height="14" fill="#EEF1F6"/>'
            f'<circle cx="{x + 20}" cy="{y + 17}" r="5.5" fill="#FF5F57"/><circle cx="{x + 38}" cy="{y + 17}" r="5.5" fill="#FEBC2E"/>'
            f'<circle cx="{x + 56}" cy="{y + 17}" r="5.5" fill="#28C840"/>'
            f'<text x="{x + w / 2 + 20}" y="{y + 22}" text-anchor="middle" font-size="13" fill="{MUTED}">{escape(title)}</text>')


def file_row(y, name, meta, color, cls=""):
    return (f'<g class="{cls}"><rect x="{FX + 18}" y="{y - 15}" width="16" height="20" rx="3" fill="{color}"/>'
            f'<text x="{FX + 44}" y="{y}" font-size="14" fill="{INK}">{escape(name)}</text>'
            f'<text x="{FX + FW - 18}" y="{y}" text-anchor="end" font-size="12.5" fill="{MUTED}">{escape(meta)}</text></g>')


# ------------------------------------------------------------------ timing
vis("copying", 4, 20)
vis("copied", 20.6, 99.4)
vis("new-mp4", 4, 99.4, rise=True)
css += (f"@keyframes progress{{0%,4%{{transform:scaleX(0)}}20%,100%{{transform:scaleX(1)}}}}\n"
        f".progress{{transform-box:fill-box;transform-origin:left;animation:progress {LOOP}s linear infinite}}\n")
vis("working", 23, 60)
vis("note", 25, 99.4)
for i in range(4):
    vis(f"r{i}", 30 + i * 7.5, 99.4, rise=True)
vis("srt", 63, 99.4, rise=True)
vis("cap0", 68, 80)
vis("cap1", 80.6, 92)
vis("cap2", 92.6, 99)
vis("say0", 1, 27)
vis("say1", 28, 62)
vis("say2", 63, 99)

folder = (f'<g filter="url(#shadow)"><rect x="{FX}" y="{FY}" width="{FW}" height="{FH}" rx="14" fill="#fff"/></g>'
          + chrome(FX, FY, FW, "Zoom Recordings")
          + f'<rect x="{FX + FW - 112}" y="{FY + 44}" width="98" height="20" rx="10" fill="#FFEDD5"/>'
          + f'<text x="{FX + FW - 63}" y="{FY + 58}" text-anchor="middle" font-size="11.5" font-weight="600" fill="#C2410C">watched</text>'
          + f'<text x="{FX + 18}" y="{FY + 58}" font-size="11.5" font-weight="700" fill="{MUTED}" letter-spacing=".04em">NAME</text>'
          + file_row(FY + 92, "kickoff.mp4", "Oct 28", "#94A3B8")
          + file_row(FY + 124, "standup.mp4", "", "#3B82F6", "new-mp4")
          + f'<g class="copying"><rect x="{FX + 190}" y="{FY + 116}" width="92" height="6" rx="3" fill="#E5E7EB"/>'
          + f'<rect class="progress" x="{FX + 190}" y="{FY + 116}" width="92" height="6" rx="3" fill="{BLUE}"/></g>'
          + f'<text class="copied" x="{FX + FW - 18}" y="{FY + 124}" text-anchor="end" font-size="12.5" fill="{MUTED}">Today</text>'
          + file_row(FY + 156, "standup.srt", "subtitles", "#F2551F", "srt"))

video = (f'<g filter="url(#shadow)"><rect x="{VX}" y="{VY}" width="{VW}" height="{VH}" rx="14" fill="#0B1020"/></g>'
         f'<rect x="{VX + 12}" y="{VY + 12}" width="{VW - 24}" height="{VH - 50}" rx="8" fill="#1E293B"/>'
         # Two people on a call, drawn simply.
         f'<circle cx="{VX + 104}" cy="{VY + 54}" r="16" fill="#475569"/><path d="M{VX + 78} {VY + 104} a26 22 0 0 1 52 0z" fill="#475569"/>'
         f'<circle cx="{VX + 196}" cy="{VY + 54}" r="16" fill="#64748B"/><path d="M{VX + 170} {VY + 104} a26 22 0 0 1 52 0z" fill="#64748B"/>'
         f'<path d="M{VX + 20} {VY + VH - 22} l9 6 -9 6z" fill="#fff"/>'
         f'<rect x="{VX + 40}" y="{VY + VH - 18}" width="{VW - 60}" height="4" rx="2" fill="#334155"/>'
         f'<rect class="progress" x="{VX + 40}" y="{VY + VH - 18}" width="{(VW - 60) * 0.4:.0f}" height="4" rx="2" fill="#F2551F"/>')
caps = ["We moved the launch to the 17th.", "Anselm owns the installer fix.", "The budget stays where it was."]
for i, c in enumerate(caps):
    cy = VY + VH - 50
    video += (f'<g class="cap{i}"><rect x="{VX + VW / 2 - 4.1 * len(c) - 10:.0f}" y="{cy - 18}" width="{8.2 * len(c) + 20:.0f}" '
              f'height="26" rx="4" fill="#000" fill-opacity=".72"/>'
              f'<text x="{VX + VW / 2}" y="{cy}" text-anchor="middle" font-size="14" fill="#fff">{escape(c)}</text></g>')

rows = [("0:00", "Morning, quick standup today."), ("0:06", "We moved the launch to the 17th."),
        ("0:11", "Anselm owns the installer fix."), ("0:15", "The budget stays where it was.")]
note = (f'<g filter="url(#shadow)"><rect x="{NX}" y="{NY}" width="{NW}" height="{NH}" rx="14" fill="#fff"/></g>'
        + chrome(NX, NY, NW, "Notes")
        + f'<g class="note"><text x="{NX + 20}" y="{NY + 70}" font-size="18" font-weight="700" fill="{INK}">standup</text>'
        + f'<text x="{NX + 20}" y="{NY + 90}" font-size="12.5" fill="{MUTED}">From standup.mp4 · on this computer</text>'
        + "".join(
            f'<g class="r{i}"><text x="{NX + 20}" y="{NY + 128 + i * 40}" font-size="12" fill="{BLUE}" {MONO}>{t}</text>'
            f'<text x="{NX + 62}" y="{NY + 128 + i * 40}" font-size="14" fill="{INK}">{escape(s)}</text></g>'
            for i, (t, s) in enumerate(rows))
        + f'<g class="srt"><rect x="{NX + 14}" y="{NY + 300}" width="{NW - 28}" height="44" rx="10" fill="#EEF4FF"/>'
        + f'<text x="{NX + 28}" y="{NY + 327}" font-size="13.5" fill="{INK}"><tspan font-weight="700" fill="{BLUE}">Subtitles saved </tspan>beside the video</text></g></g>')

wave = "".join(
    f'<rect class="bar" x="{PX + 148 + b * 8}" y="{PY + 15}" width="4" height="18" rx="2" fill="#FF9A3D"/>'
    for b in range(6))
pill = (f'<g class="working"><g filter="url(#shadow)"><rect x="{PX}" y="{PY}" width="{PW}" height="{PH}" rx="24" '
        f'fill="#17171C" fill-opacity=".94"/></g>'
        f'<circle cx="{PX + 24}" cy="{PY + 24}" r="6" fill="#F2551F"/>'
        f'<text x="{PX + 38}" y="{PY + 30}" font-size="16" font-weight="600" fill="#F4F4F5">Transcribing</text>'
        f'<g>{wave}</g>'
        f'<path class="flow" d="M{FX + FW} {FY + 120} C{FX + FW + 30} {FY + 120} {PX - 40} {PY + PH / 2} {PX} {PY + PH / 2}" '
        f'fill="none" stroke="#FF9A3D" stroke-width="3" stroke-linecap="round" stroke-dasharray="2 12"/>'
        f'<path class="flow" d="M{PX + PW} {PY + PH / 2} H{NX}" fill="none" stroke="#FF9A3D" stroke-width="3" '
        f'stroke-linecap="round" stroke-dasharray="2 12"/></g>')

css += f"""
@keyframes flow{{to{{stroke-dashoffset:-28}}}}
.flow{{animation:flow .6s linear infinite}}
@keyframes bar{{0%{{transform:scaleY(.25)}}100%{{transform:scaleY(1)}}}}
.bar{{transform-box:fill-box;transform-origin:center;animation:bar .45s ease-in-out infinite alternate}}
.bar:nth-of-type(2n){{animation-delay:-.15s}}.bar:nth-of-type(3n){{animation-delay:-.3s}}
@keyframes twinkle{{0%,100%{{opacity:.9}}50%{{opacity:.35}}}}
.star{{animation:twinkle 3s ease-in-out infinite}}
.star:nth-of-type(2n){{animation-delay:-1.4s}}.star:nth-of-type(3n){{animation-delay:-2.1s}}

/* Without motion: the finished note, the subtitles beside the video. */
@media (prefers-reduced-motion:reduce){{
  *{{animation:none!important}}
  .copying,.working,.say0,.say1,.cap1,.cap2{{opacity:0}}
  .copied,.new-mp4,.note,.r0,.r1,.r2,.r3,.srt,.cap0,.say2{{opacity:1;transform:none}}
}}
"""

SAY = f'x="{W / 2}" y="440" text-anchor="middle" font-size="19" font-style="italic" fill="#E6ECFF"'
STARS = "".join(
    f'<circle class="star" cx="{x}" cy="{y}" r="{r}" fill="#fff"/>'
    for x, y, r in [(470, 40, 2.2), (560, 120, 1.8), (400, 330, 2), (580, 380, 1.6), (940, 420, 2)])

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-labelledby="t d">
<title id="t">Watched folders and subtitles</title>
<desc id="d">A recording, standup.mp4, is copied into a folder Splashdown watches. Once it has finished copying, it is transcribed on this computer into a note line by line, with time stamps. Then standup.srt appears beside the video in the folder, and the video plays with those subtitles: "We moved the launch to the 17th."</desc>
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
  <path d="M0 458 C160 446 320 470 480 458 S800 446 960 458 V480 H0 Z" fill="url(#sea)"/>

  <!-- The watched folder, and the video with its subtitles. -->
  {folder}
  {video}

  <!-- Splashdown, transcribing. -->
  {pill}

  <!-- The note it becomes. -->
  {note}

  <text class="say0" {SAY}>A recording lands in a folder you chose…</text>
  <text class="say1" {SAY}>…and becomes a note by itself, on your computer…</text>
  <text class="say2" {SAY}>…with subtitles saved beside the video</text>
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
