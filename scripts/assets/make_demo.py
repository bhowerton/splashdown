# Generates docs/assets/demo.svg, the animated illustration at the top of
# the README: dictating, then giving a spoken command. One 12 s loop; the
# phases are percentages of it.
#
#   python3 scripts/assets/make_demo.py docs/assets/demo.svg
import sys
out = sys.argv[1]
F = 0.6  # fade length, %

def vis(name, start, end, fade=F):
    """Opacity 1 between start and end (in %), 0 elsewhere."""
    a, b = max(start - fade, 0), min(end + fade, 100)
    frames = []
    if a > 0: frames.append(f"0%,{a:.2f}%{{opacity:0}}")
    frames.append(f"{start:.2f}%,{end:.2f}%{{opacity:1}}")
    if b < 100: frames.append(f"{b:.2f}%,100%{{opacity:0}}")
    return f"@keyframes {name}{{{' '.join(frames)}}}\n.{name}{{animation:{name} 12s linear infinite}}\n"

css = ""
# Pill phases.
css += vis("p-listen", 6, 25)
css += vis("p-clean", 25.6, 30.5)
css += vis("p-inserted", 31.2, 42)
css += vis("p-command", 48, 65.5)
css += vis("p-rewrite", 66.1, 70.5)
css += vis("p-replaced", 71.2, 84)
# What the user says.
css += vis("say-a", 8, 26)
css += vis("say-b", 50, 67)

css += """
@keyframes press-opt{0%,5.9%{transform:none}6%,25%{transform:translateY(3px)}25.1%,47.9%{transform:none}48%,65.5%{transform:translateY(3px)}65.6%,100%{transform:none}}
@keyframes tint-opt{0%,5.9%{fill:#F4F6FB}6%,25%{fill:#FFD5D5}25.1%,47.9%{fill:#F4F6FB}48%,65.5%{fill:#E4DAFF}65.6%,100%{fill:#F4F6FB}}
@keyframes press-shift{0%,45.9%{transform:none}46%,47.4%{transform:translateY(3px)}47.5%,100%{transform:none}}
@keyframes tint-shift{0%,45.9%{fill:#F4F6FB}46%,47.4%{fill:#E4DAFF}47.5%,100%{fill:#F4F6FB}}
.press-opt{animation:press-opt 12s linear infinite}.tint-opt{animation:tint-opt 12s linear infinite}
.press-shift{animation:press-shift 12s linear infinite}.tint-shift{animation:tint-shift 12s linear infinite}

/* First result: hidden by a cover that slides right like typing. */
@keyframes text-a{0%,69.4%{opacity:1}70%,98.4%{opacity:0}99%,100%{opacity:1}}
.text-a{animation:text-a 12s linear infinite}
@keyframes cover-a{0%,31.2%{transform:none}37%,100%{transform:translateX(306px)}}
.cover-a{animation:cover-a 12s linear infinite, text-a 12s linear infinite}
@keyframes text-b{0%,69.9%{opacity:0}70%,94%{opacity:1}97%,100%{opacity:0}}
.text-b{animation:text-b 12s linear infinite}
@keyframes cover-b{0%,71.2%{transform:none}79%,100%{transform:translateX(426px)}}
.cover-b{animation:cover-b 12s linear infinite, text-b 12s linear infinite}
@keyframes blink{0%,49%{opacity:1}50%,100%{opacity:0}}
.caret{animation:blink 1s step-end infinite}

/* Live waveform and spinner. */
@keyframes bar{0%{transform:scaleY(.25)}100%{transform:scaleY(1)}}
.bar{transform-box:fill-box;transform-origin:center;animation:bar .45s ease-in-out infinite alternate}
.bar:nth-child(2){animation-delay:-.15s}.bar:nth-child(3){animation-delay:-.3s}
.bar:nth-child(4){animation-delay:-.05s}.bar:nth-child(5){animation-delay:-.25s}
.bar:nth-child(6){animation-delay:-.4s}.bar:nth-child(7){animation-delay:-.1s}
@keyframes spin{to{transform:rotate(360deg)}}
.spin{transform-box:fill-box;transform-origin:center;animation:spin .8s linear infinite}
@keyframes twinkle{0%,100%{opacity:.9}50%{opacity:.35}}
.star{animation:twinkle 3s ease-in-out infinite}
.star:nth-of-type(2n){animation-delay:-1.4s}.star:nth-of-type(3n){animation-delay:-2.1s}

@media (prefers-reduced-motion:reduce){
  *{animation:none!important}
  .p-listen,.p-clean,.p-command,.p-rewrite,.p-replaced,.say-a,.say-b,.text-a,.cover-a,.cover-b{opacity:0}
  .p-inserted,.text-b{opacity:1}
}
"""

def wave(x0, color):
    return "<g>" + "".join(
        f'<rect class="bar" x="{x0 + i*10}" y="342" width="4" height="26" rx="2" fill="{color}"/>'
        for i in range(7)) + "</g>"

def pill(cls, x, w, inner):
    return (f'<g class="{cls}"><rect x="{x}" y="330" width="{w}" height="50" rx="25" '
            f'fill="#17171C" fill-opacity=".94" filter="url(#shadow)"/>{inner}</g>')

LABEL = 'fill="#F4F4F5" font-size="17" font-weight="600"'
HINT = 'fill="#F4F4F5" fill-opacity=".6" font-size="13"'
RED, PURPLE, GREEN = "#F04545", "#9473FA", "#52CF66"

def check(cx):
    return f'<path d="M{cx-7} 355 l5 5 l10 -11" fill="none" stroke="{GREEN}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>'

def spinner(cx):
    return (f'<g><circle cx="{cx}" cy="355" r="9" fill="none" stroke="#F4F4F5" stroke-opacity=".25" stroke-width="3"/>'
            f'<path class="spin" d="M{cx} 346 a9 9 0 0 1 9 9" fill="none" stroke="#F4F4F5" stroke-width="3" stroke-linecap="round"/></g>')

pills = "".join([
    pill("p-listen", 290, 380,
         f'<circle cx="316" cy="355" r="6" fill="{RED}"/><text x="331" y="361" {LABEL}>Listening</text>'
         + wave(425, RED) + f'<text x="508" y="360" {HINT}>⇧ command · esc cancel</text>'),
    pill("p-clean", 375, 210, spinner(402) + f'<text x="421" y="361" {LABEL}>Cleaning up…</text>'),
    pill("p-inserted", 385, 190, check(410) + f'<text x="428" y="361" {LABEL}>Inserted</text><text x="510" y="360" {HINT}>0.9 s</text>'),
    pill("p-command", 290, 380,
         f'<circle cx="316" cy="355" r="6" fill="{PURPLE}"/><text x="331" y="361" {LABEL}>Command</text>'
         + wave(425, PURPLE) + f'<text x="508" y="360" {HINT}>release to apply</text>'),
    pill("p-rewrite", 385, 190, spinner(412) + f'<text x="431" y="361" {LABEL}>Rewriting…</text>'),
    pill("p-replaced", 380, 200, check(405) + f'<text x="423" y="361" {LABEL}>Replaced</text><text x="510" y="360" {HINT}>1.1 s</text>'),
])

SAY = 'x="480" y="428" text-anchor="middle" font-size="19" font-style="italic" fill="#E6ECFF"'
FIELD = 'font-size="22" fill="#111827"'

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="960" height="480" viewBox="0 0 960 480" role="img" aria-labelledby="t d">
<title id="t">Splashdown demo</title>
<desc id="d">Holding the Option key and saying "um, so can we meet Tuesday… actually Thursday at 3" types "Can we meet Thursday at 3?" into an email. Tapping Shift and saying "make it more formal" rewrites it to "Would Thursday at 3 p.m. work for you?"</desc>
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
  <circle class="star" cx="70" cy="60" r="2.5" fill="#fff"/>
  <circle class="star" cx="880" cy="90" r="3" fill="#fff"/>
  <circle class="star" cx="110" cy="210" r="2" fill="#fff"/>
  <circle class="star" cx="860" cy="230" r="2" fill="#fff"/>
  <circle class="star" cx="40" cy="300" r="1.8" fill="#fff"/>
  <circle class="star" cx="920" cy="330" r="2.2" fill="#fff"/>
  <path d="M0 452 C160 438 320 466 480 452 S800 438 960 452 V480 H0 Z" fill="url(#sea)"/>

  <!-- The app being dictated into. -->
  <g filter="url(#shadow)"><rect x="150" y="40" width="660" height="206" rx="14" fill="#fff"/></g>
  <g clip-path="url(#window)">
    <rect x="150" y="40" width="660" height="40" fill="#EEF1F6"/>
    <circle cx="174" cy="60" r="6" fill="#FF5F57"/><circle cx="194" cy="60" r="6" fill="#FEBC2E"/><circle cx="214" cy="60" r="6" fill="#28C840"/>
    <text x="480" y="65" text-anchor="middle" font-size="14" fill="#6B7280">New message</text>
    <text x="176" y="114" font-size="16" fill="#9CA3AF">To:</text>
    <text x="206" y="114" font-size="16" fill="#374151">Sam Rivera</text>
    <line x1="170" y1="130" x2="790" y2="130" stroke="#E5E7EB"/>
    <text class="text-a" x="176" y="182" textLength="300" lengthAdjust="spacing" {FIELD}>Can we meet Thursday at 3?</text>
    <text class="text-b" x="176" y="182" textLength="420" lengthAdjust="spacing" {FIELD}>Would Thursday at 3 p.m. work for you?</text>
    <g class="cover-a"><rect x="174" y="156" width="620" height="36" fill="#fff"/><rect class="caret" x="174" y="160" width="2" height="28" fill="#2563EB"/></g>
    <g class="cover-b"><rect x="174" y="156" width="620" height="36" fill="#fff"/><rect class="caret" x="174" y="160" width="2" height="28" fill="#2563EB"/></g>
  </g>

  <!-- The keys being pressed. -->
  <g font-weight="600" font-size="22" fill="#1F2937" text-anchor="middle">
    <g class="press-shift"><rect class="tint-shift" x="54" y="330" width="52" height="48" rx="10" fill="#F4F6FB"/><text x="80" y="361">⇧</text></g>
    <g class="press-opt"><rect class="tint-opt" x="118" y="330" width="52" height="48" rx="10" fill="#F4F6FB"/><text x="144" y="361">⌥</text></g>
  </g>
  <text x="80" y="398" text-anchor="middle" font-size="12" fill="#C7D2FE">tap</text>
  <text x="144" y="398" text-anchor="middle" font-size="12" fill="#C7D2FE">hold</text>

  <!-- The overlay pill. -->
  {pills}

  <!-- What the user says. -->
  <text class="say-a" {SAY}>“um, so can we meet Tuesday… actually Thursday at 3”</text>
  <text class="say-b" {SAY}>“make it more formal”</text>
</g>
</svg>
'''
open(out, "w").write(svg)
print(len(svg), "bytes")
