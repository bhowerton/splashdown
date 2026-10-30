"use strict";

// Insights tab: words dictated, speaking speed, time saved, streak, a
// 30-day chart and the apps words went to (insights.rs). Local numbers only.

async function loadInsights() {
  let s;
  try {
    s = await invoke("get_insights");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  const hours = s.minutes_saved / 60;
  const saved = hours >= 1 ? `${hours.toFixed(1)} h` : `${Math.round(s.minutes_saved)} min`;
  const cards = [
    [s.week_words.toLocaleString(), "words this week", `${s.today_words.toLocaleString()} today · ${s.total_words.toLocaleString()} in all`],
    [s.speaking_wpm ? `${Math.round(s.speaking_wpm)}` : "—", "words per minute", `Typing is about ${s.typing_wpm}`],
    [saved, "saved", "Compared with typing the same words"],
    [String(s.streak_days), s.streak_days === 1 ? "day streak" : "day streak", "Days in a row you've dictated"],
  ];
  $("ins-stats").replaceChildren(
    ...cards.map(([value, label, detail]) => {
      const c = document.createElement("div");
      c.className = "stat";
      const v = document.createElement("b");
      v.textContent = value;
      const l = document.createElement("span");
      l.textContent = label;
      const d = document.createElement("small");
      d.textContent = detail;
      c.append(v, l, d);
      return c;
    })
  );

  const max = Math.max(1, ...s.chart.map((d) => d.words));
  const chart = $("ins-chart");
  chart.setAttribute("role", "img");
  chart.setAttribute("aria-label", `Words per day for the last 30 days: ${s.chart.reduce((n, d) => n + d.words, 0).toLocaleString()} in all, at most ${max.toLocaleString()} in a day.`);
  chart.replaceChildren(
    ...s.chart.map((d) => {
      const bar = document.createElement("span");
      bar.className = "bar";
      bar.style.height = `${Math.max(d.words ? 4 : 1, (d.words / max) * 100)}%`;
      const when = new Date(`${d.date}T12:00`).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
      bar.title = `${when}: ${d.words.toLocaleString()} words`;
      return bar;
    })
  );

  const top = s.top_apps[0]?.[1] ?? 1;
  $("ins-apps").replaceChildren(
    ...(s.top_apps.length
      ? s.top_apps.map(([app, words]) => {
          const r = document.createElement("div");
          r.className = "row app-share";
          const name = document.createElement("span");
          name.className = "lbl";
          name.textContent = app;
          const meter = document.createElement("span");
          meter.className = "share";
          const fill = document.createElement("i");
          fill.style.width = `${(words / top) * 100}%`;
          meter.append(fill);
          const n = document.createElement("span");
          n.className = "mono";
          n.textContent = words.toLocaleString();
          r.append(name, meter, n);
          return r;
        })
      : [Object.assign(document.createElement("div"), { className: "row", innerHTML: '<span class="lbl">Nothing yet<small>Dictate into a few apps and they show up here.</small></span>' })])
  );
}

$("ins-clear").addEventListener("click", async () => {
  const sure = await askConfirm("Reset your usage stats?", { action: "Reset", detail: "Words, speaking time and app counts start again from zero." });
  if (!sure) return;
  await invoke("clear_insights").catch((e) => toast(String(e), true));
  loadInsights();
});
