"use strict";

// The call offer (`calls.rs`): record a meeting when a call app starts
// using the microphone, or stop when the recorded call ends; and, for a
// meeting on the calendar that's about to start (`calls/calendar.rs`),
// join it and record. Branded, so it's clear whose it is, with the
// calendar event (when known), what recording will do with the settings
// as they are, and how long it stays.

/** `{ offer: { record: { app } } | { stop: { app } } | { join: { service } },
 *  event, starts, joined, summary, recorded, seconds }`, or null. The link
 *  Join opens stays in the app. */
let panel = null;

/** When a meeting starts, to end "Your meeting …": "starts in 2 minutes",
 *  "is starting", "started 3 minutes ago". */
function when(starts) {
  const minutes = Math.round((starts - Date.now()) / 60000);
  if (minutes > 0) return `starts in ${minutes === 1 ? "a minute" : `${minutes} minutes`}`;
  if (minutes < 0) return `started ${minutes === -1 ? "a minute" : `${-minutes} minutes`} ago`;
  return "is starting";
}

function renderTitle() {
  const { record, stop, join } = panel?.offer ?? {};
  const app = (record ?? stop)?.app ?? "A call app";
  $("of-title").textContent = join
    ? `Your meeting ${when(panel.starts)}`
    : stop
      ? `The ${app} call seems to have ended`
      : `${app} is using the microphone`;
}

function render() {
  const { record, stop, join } = panel?.offer ?? {};
  const app = (record ?? stop)?.app ?? "A call app";
  renderTitle();

  const event = record || join ? panel?.event : null;
  $("of-event").hidden = !event;
  if (event) {
    const people = event.people?.length ? ` · with ${event.people.join(", ")}` : "";
    $("of-event").textContent = `${event.title || "(No title)"}${people}`;
  }

  $("of-text").textContent = stop
    ? `${panel?.recorded ? `${panel.recorded} recorded. ` : ""}Stop and finish the meeting note?`
    : join
      ? `It's on ${join.service}. Record it into your notes?`
      : "Record this meeting into your notes?";

  // What recording will do, and where to change it.
  const recording = record || join;
  const summary = recording ? panel?.summary ?? [] : [];
  $("of-summary").hidden = !recording;
  if (recording) {
    const change = document.createElement("button");
    change.className = "link";
    change.textContent = "Change…";
    change.addEventListener("click", () => answer("settings"));
    const said = summary.join(" · ");
    $("of-summary").replaceChildren(...(said ? [`${said} · `] : []), change);
  }
  $("of-remind").hidden = !recording;

  $("of-yes").textContent = stop ? "Stop recording" : "Record";
  $("of-no").textContent = stop ? "Keep recording" : "Not now";
  // Join comes first; once it's clicked, Record is what's left.
  $("of-join").hidden = !join;
  $("of-join").classList.toggle("primary", Boolean(join && !panel.joined));
  $("of-yes").classList.toggle("primary", !join || Boolean(panel.joined));
  $("of-never").hidden = !record;
  $("of-never").textContent = `Don't offer for ${app}`;

  // How long until it goes away by itself.
  const left = $("of-left");
  left.style.animation = "none";
  void left.offsetWidth;
  left.style.animation = panel ? `of-left ${panel.seconds ?? 45}s linear forwards` : "none";
}

function answer(choice) {
  invoke("call_offer_answer", { answer: choice }).catch((e) => console.warn(e));
}

$("of-yes").addEventListener("click", () => answer(panel?.offer?.stop ? "stop" : "record"));
$("of-join").addEventListener("click", () => answer("join"));
$("of-no").addEventListener("click", () => answer("dismiss"));
$("of-never").addEventListener("click", () => answer("never"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") answer("dismiss");
});
// A meeting's offer stays up for minutes: "starts in 2 minutes" keeps time.
setInterval(() => {
  if (panel?.offer?.join) renderTitle();
}, 10000);

listen("call-offer-changed", (event) => {
  panel = event.payload;
  render();
});
invoke("call_offer_state")
  .then((state) => {
    panel = state;
    render();
  })
  .catch(() => render());
