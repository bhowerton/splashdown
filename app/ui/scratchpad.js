"use strict";

// The scratchpad window: one note to dictate into, saved as you type, plus
// the last 20 earlier notes. Kept in scratchpad.json on this computer.

/** How long typing pauses before the note is saved (ms). */
const SAVE_DELAY = 400;

const note = $("sp-note");
let saveTimer = null;
/** The text last sent to be saved, so unchanged notes aren't re-saved. */
let savedText = "";
/** The app "Send to" pastes into, or null. */
let sendTarget = null;
let earlierNotes = [];

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveNow, SAVE_DELAY);
  updateButtons();
}

function saveNow() {
  clearTimeout(saveTimer);
  saveTimer = null;
  if (note.value === savedText) return;
  savedText = note.value;
  invoke("scratchpad_save", { text: savedText }).catch((e) => toast(String(e), true));
}

/** Shows what the backend returned after New note, Open or Delete. */
function showNotes(notes) {
  note.value = notes.current.text;
  savedText = note.value;
  earlierNotes = notes.earlier;
  renderEarlier();
  updateButtons();
}

function updateButtons() {
  const empty = note.value.trim() === "";
  $("sp-copy").disabled = empty;
  $("sp-new").disabled = empty;
  const send = $("sp-send");
  send.textContent = sendTarget ? `Send to ${sendTarget}` : "Send";
  send.disabled = empty || !sendTarget;
  send.title = sendTarget
    ? `Paste the note into ${sendTarget} (${navigator.platform.startsWith("Mac") ? "⌘" : "Ctrl+"}Return)`
    : "Open the scratchpad from another app to send the note there";
  const count = earlierNotes.length;
  $("sp-show-earlier").textContent = count ? `Earlier notes · ${count}` : "Earlier notes";
}

function focusNote() {
  if ($("sp-editor").hidden) return;
  note.focus();
  // Carry on at the end, where the next dictation should go.
  note.setSelectionRange(note.value.length, note.value.length);
}

// ---------------------------------------------------------------- actions

async function copyText(text) {
  try {
    await invoke("copy_text", { text });
    toast("Copied");
  } catch (e) {
    toast(String(e), true);
  }
}

async function newNote() {
  clearTimeout(saveTimer);
  try {
    showNotes(await invoke("scratchpad_new", { text: note.value }));
    focusNote();
  } catch (e) {
    toast(String(e), true);
  }
}

async function sendNote() {
  if ($("sp-send").disabled) return;
  saveNow();
  try {
    // Hides this window, switches to the app and pastes. On failure the
    // window comes back with the note on the clipboard.
    await invoke("scratchpad_send", { text: note.value });
  } catch (e) {
    toast(String(e), true);
  }
}

function showEarlier(show) {
  $("sp-editor").hidden = show;
  $("sp-earlier-view").hidden = !show;
  if (show) {
    saveNow();
    renderEarlier();
    $("sp-back").focus();
  } else {
    focusNote();
  }
}

// ---------------------------------------------------------------- earlier notes

/** "2:02 PM" today, "Oct 28" this year, "Oct 28, 2025" before. */
function when(ms) {
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  }
  const opts = { month: "short", day: "numeric" };
  if (d.getFullYear() !== now.getFullYear()) opts.year = "numeric";
  return d.toLocaleDateString([], opts);
}

function renderEarlier() {
  const list = $("sp-earlier");
  list.replaceChildren(...earlierNotes.map(earlierRow));
  $("sp-empty").hidden = earlierNotes.length > 0;
  updateButtons();
}

function earlierRow(n) {
  const li = document.createElement("li");
  const text = document.createElement("div");
  text.className = "sp-text";
  text.textContent = n.text;
  const meta = document.createElement("div");
  meta.className = "sp-meta";
  const time = document.createElement("time");
  time.dateTime = new Date(n.at).toISOString();
  time.textContent = when(n.at);
  const open = button("Open", async () => {
    clearTimeout(saveTimer);
    try {
      showNotes(await invoke("scratchpad_open_note", { at: n.at, text: note.value }));
      showEarlier(false);
    } catch (e) {
      toast(String(e), true);
    }
  }, "link");
  open.setAttribute("aria-label", `Open note from ${time.textContent}`);
  const copy = button("Copy", () => copyText(n.text), "link");
  copy.setAttribute("aria-label", `Copy note from ${time.textContent}`);
  const del = button("Delete", async () => {
    try {
      const notes = await invoke("scratchpad_delete_note", { at: n.at });
      earlierNotes = notes.earlier;
      renderEarlier();
    } catch (e) {
      toast(String(e), true);
    }
  }, "link danger");
  del.setAttribute("aria-label", `Delete note from ${time.textContent}`);
  meta.append(time, open, copy, del);
  li.append(text, meta);
  return li;
}

// ---------------------------------------------------------------- wiring

note.addEventListener("input", scheduleSave);
// Hiding the window (⌃⌥N, Send, close) takes focus away: save first.
window.addEventListener("blur", saveNow);
document.addEventListener("visibilitychange", saveNow);
window.addEventListener("focus", () => {
  if (document.activeElement !== note) focusNote();
});
note.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    sendNote();
  }
});

$("sp-copy").addEventListener("click", () => copyText(note.value));
$("sp-new").addEventListener("click", newNote);
$("sp-send").addEventListener("click", sendNote);
$("sp-show-earlier").addEventListener("click", () => showEarlier(true));
$("sp-back").addEventListener("click", () => showEarlier(false));

// Shown again (⌃⌥N or the menu bar), maybe from a different app.
listen("scratchpad-shown", (event) => {
  sendTarget = event.payload ?? null;
  updateButtons();
  focusNote();
});

(async () => {
  try {
    const view = await invoke("scratchpad_load");
    sendTarget = view.target ?? null;
    showNotes(view.notes);
  } catch (e) {
    toast(String(e), true);
  }
  focusNote();
})();

// ---------------------------------------------------------------- preview

/** Sample backend for opening this page as a plain file:// page (see
 *  invoke in common.js). */
async function mockInvoke(cmd, args) {
  const hour = 3600 * 1000;
  const sample = {
    current: { text: "Ideas for the launch post:\n- lead with the hold-to-talk demo\n- mention it's free and open source", at: Date.now() },
    earlier: [
      { text: "Call the dentist about moving Thursday's appointment.", at: Date.now() - 2 * hour },
      { text: "Groceries: oat milk, lemons, coffee beans, rice.", at: Date.now() - 30 * hour },
      { text: "Reply to Sam: the Windows build compiles but hasn't been run yet. I'll test it on Monday and send notes.", at: Date.now() - 400 * hour },
    ],
  };
  switch (cmd) {
    case "scratchpad_load":
      return { notes: sample, target: "Mail" };
    case "scratchpad_new":
    case "scratchpad_open_note":
    case "scratchpad_delete_note":
      return sample;
    default:
      return null;
  }
}
