"use strict";

// The call offer (`calls.rs`): record a meeting when a call app starts
// using the microphone, or stop when the recorded call ends. Branded, so
// it's clear whose it is, with the calendar event (when known), what
// recording will do with the settings as they are, and how long it stays.

/** `{ offer: { record: { app } } | { stop: { app } }, event, summary,
 *  recorded, seconds }`, or null. */
let panel = null;

function render() {
  const record = panel?.offer?.record;
  const stop = panel?.offer?.stop;
  const app = (record ?? stop)?.app ?? "A call app";
  $("of-title").textContent = stop ? `The ${app} call seems to have ended` : `${app} is using the microphone`;

  const event = record ? panel?.event : null;
  $("of-event").hidden = !event;
  if (event) {
    const people = event.people?.length ? ` · with ${event.people.join(", ")}` : "";
    $("of-event").textContent = `${event.title}${people}`;
  }

  $("of-text").textContent = stop
    ? `${panel?.recorded ? `${panel.recorded} recorded. ` : ""}Stop and finish the meeting note?`
    : "Record this meeting into your notes?";

  // What recording will do, and where to change it.
  const summary = record ? panel?.summary ?? [] : [];
  $("of-summary").hidden = !record;
  if (record) {
    const change = document.createElement("button");
    change.className = "link";
    change.textContent = "Change…";
    change.addEventListener("click", () => answer("settings"));
    const said = summary.join(" · ");
    $("of-summary").replaceChildren(...(said ? [`${said} · `] : []), change);
  }
  $("of-remind").hidden = !record;

  $("of-yes").textContent = stop ? "Stop recording" : "Record";
  $("of-no").textContent = stop ? "Keep recording" : "Not now";
  $("of-never").hidden = Boolean(stop);
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
$("of-no").addEventListener("click", () => answer("dismiss"));
$("of-never").addEventListener("click", () => answer("never"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") answer("dismiss");
});

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
