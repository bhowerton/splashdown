"use strict";

// The meeting pill (`pill.rs`): the same status the Notes window's meeting
// bar shows, twice a second.

function clockText(seconds) {
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

function show(status) {
  if (!status) return;
  const finishing = status.finishing;
  $("pl-text").textContent = finishing
    ? "Finishing the note…"
    : status.warning
      ? "Still in a meeting?"
      : `Recording · ${clockText(status.elapsed)}${status.them_problem ? " · your side only" : ""}`;
  $("pl-text").title = status.warning ?? status.them_problem ?? "";
  document.querySelector(".pl").classList.toggle("finishing", finishing);
  $("pl-keep").hidden = !status.warning || finishing;
  $("pl-stop").hidden = finishing;
  $("pl-levels").hidden = finishing;
  const level = (v) => `${Math.min(100, Math.round(Math.sqrt(v) * 140))}%`;
  $("pl-you").style.width = level(status.you);
  $("pl-them").style.width = level(status.them);
}

$("pl-stop").addEventListener("click", () => invoke("meeting_toggle").catch((e) => console.warn(e)));
$("pl-keep").addEventListener("click", () => invoke("meeting_keep_going").catch(() => {}));
$("pl-notes").addEventListener("click", () => invoke("open_notes").catch(() => {}));

listen("meeting-status", (event) => show(event.payload));
invoke("meeting_status").then(show).catch(() => {});
