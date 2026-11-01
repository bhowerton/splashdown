"use strict";

// The Ask panel: a spoken question and its answer, streamed in by the Rust
// side (`ask.rs`). Insert pastes the answer into the app the question was
// asked from; Esc closes.

/** What the panel shows (`ask::Shown`). */
let shown = { question: "", answer: "", done: false, error: null, app: null, notes: 0 };

function render() {
  $("ak-question").textContent = shown.question ? `“${shown.question}”` : "";
  const answer = $("ak-answer");
  answer.classList.toggle("waiting", !shown.answer && !shown.error);
  answer.classList.toggle("failed", Boolean(shown.error));
  answer.textContent =
    shown.error || shown.answer || (shown.done ? "No answer came back. Try asking again." : "Thinking…");
  const ready = shown.done && !shown.error && Boolean(shown.answer);
  $("ak-copy").disabled = !ready;
  $("ak-insert").disabled = !ready || !shown.app;
  $("ak-insert").textContent = shown.app ? `Insert into ${shown.app}` : "Insert";
  const notes = shown.notes ? `Used ${shown.notes} matching note${shown.notes === 1 ? "" : "s"}` : "";
  $("ak-note").textContent = notes;
  if (!shown.done) answer.scrollTop = answer.scrollHeight;
}

async function insert() {
  if ($("ak-insert").disabled) return;
  try {
    await invoke("ask_insert", { text: shown.answer });
  } catch (e) {
    toast(String(e), true);
  }
}

async function copy() {
  if ($("ak-copy").disabled) return;
  try {
    await invoke("ask_copy", { text: shown.answer });
    toast("Copied");
  } catch (e) {
    toast(String(e), true);
  }
}

$("ak-insert").addEventListener("click", insert);
$("ak-copy").addEventListener("click", copy);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("ask_close");
  } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    insert();
  } else if (e.key === "c" && (e.metaKey || e.ctrlKey) && !window.getSelection()?.toString()) {
    // ⌘C with nothing selected copies the whole answer.
    e.preventDefault();
    copy();
  }
});

listen("ask-changed", (event) => {
  shown = event.payload;
  render();
});
invoke("ask_state")
  .then((state) => {
    shown = state;
    render();
  })
  .catch(() => render());
