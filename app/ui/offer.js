"use strict";

// The call offer (`calls.rs`): record a meeting when a call app starts
// using the microphone, or stop when the recorded call ends.

/** `{ record: { app } }` or `{ stop: { app } }`, or null. */
let offer = null;

function render() {
  const record = offer?.record;
  const stop = offer?.stop;
  const app = (record ?? stop)?.app ?? "A call app";
  $("of-title").textContent = stop ? `The ${app} call seems to have ended` : `${app} is using the microphone`;
  $("of-text").textContent = stop
    ? "Stop recording and finish the meeting note?"
    : "Record this meeting? The transcript goes into your notes as the call goes.";
  $("of-yes").textContent = stop ? "Stop recording" : "Record";
  $("of-no").textContent = stop ? "Keep recording" : "Not now";
  $("of-never").hidden = Boolean(stop);
}

function answer(choice) {
  invoke("call_offer_answer", { answer: choice }).catch((e) => console.warn(e));
}

$("of-yes").addEventListener("click", () => answer(offer?.stop ? "stop" : "record"));
$("of-no").addEventListener("click", () => answer("dismiss"));
$("of-never").addEventListener("click", () => answer("never"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") answer("dismiss");
});

listen("call-offer-changed", (event) => {
  offer = event.payload;
  render();
});
invoke("call_offer_state")
  .then((state) => {
    offer = state;
    render();
  })
  .catch(() => render());
