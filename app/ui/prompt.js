"use strict";

// A question the app asks on its own (`prompt.rs`): a meeting's summary,
// finishing one after a crash, quitting mid-meeting.

function show(prompt) {
  if (!prompt) return;
  $("pr-title").textContent = prompt.title;
  $("pr-text").textContent = prompt.text;
  // The usual answer (the first) goes on the right.
  const buttons = prompt.buttons.map((label, i) =>
    button(label, () => invoke("prompt_answer", { index: i }), i === 0 ? "btn primary" : "btn"),
  );
  $("pr-acts").replaceChildren(...buttons.reverse());
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("prompt_answer", { index: null });
});

listen("prompt-changed", (event) => show(event.payload));
invoke("prompt_state").then(show).catch(() => {});
