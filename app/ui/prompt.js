"use strict";

// A question the app asks on its own (`prompt.rs`): a meeting's summary,
// finishing one after a crash, quitting mid-meeting.

function show(prompt) {
  if (!prompt) return;
  $("pr-title").textContent = prompt.title;
  $("pr-text").textContent = prompt.text;
  $("pr-text").hidden = !prompt.text;
  // Choices to pick from (a language), answered by their position.
  const choices = prompt.choices ?? [];
  $("pr-choices").hidden = choices.length === 0;
  $("pr-choices").replaceChildren(
    ...choices.map((label, i) => button(label, () => invoke("prompt_answer", { index: i }), "btn")),
  );
  // The usual answer (the first) goes on the right; with choices, none is
  // the usual one.
  const buttons = prompt.buttons.map((label, i) =>
    button(
      label,
      () => invoke("prompt_answer", { index: choices.length + i }),
      i === 0 && choices.length === 0 ? "btn primary" : "btn",
    ),
  );
  $("pr-acts").replaceChildren(...buttons.reverse());
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("prompt_answer", { index: null });
});

listen("prompt-changed", (event) => show(event.payload));
invoke("prompt_state").then(show).catch(() => {});
