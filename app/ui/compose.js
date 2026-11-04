"use strict";

// The Compose panel: a draft written from what the user said they want
// to write, streamed in by the Rust side (`compose.rs`). While it's open,
// a dictation changes the draft; so do the quick changes and a typed
// change. ‹ › go between versions; Insert pastes the one shown into the
// app it's for; Esc closes.

/** What the panel shows (`compose::Shown`). */
let shown = { versions: [], at: 0, writing: false, error: null, app: null, replying: false, read_window: false, removed: false };
/** Something is being read aloud (`reading.rs`). */
let reading = false;
/** The dictation key's name, for the hint. */
let dictKey = "the dictation key";

function current() {
  return shown.versions[shown.at] ?? { said: "", text: "" };
}

function render() {
  const version = current();
  const ready = !shown.writing && !shown.error && Boolean(version.text);
  $("cp-said").textContent = version.said ? `“${version.said}”` : "";
  $("cp-removed").hidden = !shown.removed;
  const source = shown.replying
    ? "Answering the text you selected"
    : shown.read_window
      ? "Using the conversation on screen"
      : "";
  $("cp-source").textContent = source;
  $("cp-source").hidden = !source;
  const draft = $("cp-draft");
  draft.classList.toggle("waiting", !version.text && !shown.error);
  draft.classList.toggle("failed", Boolean(shown.error));
  draft.textContent = shown.error || version.text || (shown.writing ? "Writing…" : "");
  document.querySelectorAll(".cp-chip").forEach((chip) => (chip.disabled = !ready));
  $("cp-change-input").disabled = !ready;
  $("cp-copy").disabled = !ready;
  $("cp-insert").disabled = !ready || !shown.app;
  $("cp-insert").textContent = shown.app ? `Insert into ${shown.app}` : "Insert";
  $("cp-read").disabled = !reading && !ready;
  $("cp-read").textContent = reading ? "Stop reading" : "Read aloud";
  $("cp-read").setAttribute("aria-pressed", String(reading));
  const many = shown.versions.length > 1;
  $("cp-versions").hidden = !many;
  $("cp-ver").textContent = `${shown.at + 1} of ${shown.versions.length}`;
  $("cp-prev").disabled = shown.writing || shown.at === 0;
  $("cp-next").disabled = shown.writing || shown.at >= shown.versions.length - 1;
  const hint = $("cp-hint");
  const key = document.createElement("kbd");
  key.textContent = dictKey;
  hint.replaceChildren("Hold ", key, " and say a change");
}

async function change(text) {
  if (!text.trim()) return;
  try {
    await invoke("compose_change", { change: text });
  } catch (e) {
    toast(String(e), true);
  }
}

document.querySelectorAll(".cp-chip").forEach((chip) => chip.addEventListener("click", () => change(chip.dataset.change)));

$("cp-change").addEventListener("submit", (e) => {
  e.preventDefault();
  const input = $("cp-change-input");
  const text = input.value;
  input.value = "";
  change(text);
});

$("cp-prev").addEventListener("click", () => invoke("compose_version", { at: shown.at - 1 }));
$("cp-next").addEventListener("click", () => invoke("compose_version", { at: shown.at + 1 }));

async function insert() {
  if ($("cp-insert").disabled) return;
  try {
    await invoke("compose_insert", { text: current().text });
  } catch (e) {
    toast(String(e), true);
  }
}

async function copy() {
  if ($("cp-copy").disabled) return;
  try {
    await invoke("compose_copy", { text: current().text });
    toast("Copied");
  } catch (e) {
    toast(String(e), true);
  }
}

$("cp-insert").addEventListener("click", insert);
$("cp-copy").addEventListener("click", copy);
$("cp-read").addEventListener("click", async () => {
  if ($("cp-read").disabled) return;
  try {
    reading = await invoke("reading_toggle", { text: current().text });
    render();
  } catch (e) {
    toast(String(e), true);
  }
});

document.addEventListener("keydown", (e) => {
  const typing = document.activeElement === $("cp-change-input");
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("compose_close");
  } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    insert();
  } else if (e.key === "c" && (e.metaKey || e.ctrlKey) && !typing && !window.getSelection()?.toString()) {
    // ⌘C with nothing selected copies the whole draft.
    e.preventDefault();
    copy();
  }
});

listen("reading-changed", (event) => {
  reading = Boolean(event.payload);
  render();
});
listen("compose-changed", (event) => {
  shown = event.payload;
  render();
});
invoke("get_state")
  .then((state) => {
    dictKey = dictationKeyLabel(state);
    render();
  })
  .catch(() => {});
invoke("compose_state")
  .then((state) => {
    shown = state;
    render();
  })
  .catch(() => render());
invoke("reading_state")
  .then((on) => {
    reading = Boolean(on);
    render();
  })
  .catch(() => {});
