"use strict";

// The Ask panel: a spoken question and its answer, streamed in by the Rust
// side (`ask.rs`). Insert pastes the answer into the app the question was
// asked from; Esc closes.

/** What the panel shows (`ask::Shown`). */
let shown = { earlier: [], question: "", answer: "", done: false, error: null, app: null, notes: 0 };
/** Something is being read aloud (`reading.rs`). */
let reading = false;

function renderRead() {
  const button = $("ak-read");
  const ready = shown.done && !shown.error && Boolean(shown.answer);
  button.disabled = !reading && !ready;
  button.textContent = reading ? "Stop reading" : "Read aloud";
  button.setAttribute("aria-pressed", String(reading));
}

function render() {
  const earlier = $("ak-earlier");
  earlier.replaceChildren(
    ...(shown.earlier ?? []).map((turn) => {
      const block = document.createElement("div");
      block.className = "ak-earlier";
      const q = document.createElement("p");
      q.className = "ak-question";
      q.textContent = `“${turn.question}”`;
      const a = document.createElement("div");
      a.className = "ak-answer";
      a.textContent = turn.answer;
      block.append(q, a);
      return block;
    }),
  );
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
  renderRead();
  const follow = $("ak-follow-input");
  follow.disabled = !shown.done;
  if (!shown.done) $("ak-scroll").scrollTop = $("ak-scroll").scrollHeight;
}

$("ak-follow").addEventListener("submit", async (e) => {
  e.preventDefault();
  const input = $("ak-follow-input");
  const question = input.value.trim();
  if (!question) return;
  input.value = "";
  try {
    await invoke("ask_followup", { question });
  } catch (err) {
    toast(String(err), true);
  }
});

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

async function readAloud() {
  if ($("ak-read").disabled) return;
  try {
    reading = await invoke("reading_toggle", { text: shown.answer });
    renderRead();
  } catch (e) {
    toast(String(e), true);
  }
}

$("ak-read").addEventListener("click", readAloud);
$("ak-insert").addEventListener("click", insert);
$("ak-copy").addEventListener("click", copy);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("ask_close");
  } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && document.activeElement !== $("ak-follow-input")) {
    e.preventDefault();
    insert();
  } else if (
    e.key === "c" &&
    (e.metaKey || e.ctrlKey) &&
    document.activeElement !== $("ak-follow-input") &&
    !window.getSelection()?.toString()
  ) {
    // ⌘C with nothing selected copies the whole answer.
    e.preventDefault();
    copy();
  }
});

listen("reading-changed", (event) => {
  reading = Boolean(event.payload);
  renderRead();
});
invoke("reading_state")
  .then((on) => {
    reading = Boolean(on);
    renderRead();
  })
  .catch(() => {});
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
