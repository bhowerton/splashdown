"use strict";

// Connecting the window's controls and events; starts the window. Loaded after every other notes/ file.

function toggleList(show = $("nt-side").hidden) {
  $("nt-side").hidden = !show;
  // Just the note: folders and tags go too, and come back as they were.
  let navShown = true;
  try {
    navShown = localStorage.getItem("notes.nav") !== "hidden";
  } catch {
    // Shown.
  }
  $("nt-nav").hidden = !show || !navShown || matchMedia("(max-width: 900px)").matches;
  $("nt-toggle").setAttribute("aria-expanded", String(show));
  try {
    localStorage.setItem("notes.list", show ? "shown" : "hidden");
  } catch {
    // Not remembered; fine.
  }
}

// Hiding the window (⌃⌥N, Paste into, close) takes focus away: save first.
window.addEventListener("blur", () => {
  saveNow();
  flushLiveNotes();
});
document.addEventListener("visibilitychange", () => {
  saveNow();
  flushLiveNotes();
});
window.addEventListener("focus", () => {
  refresh();
  if (document.activeElement === document.body) focusNote();
});
setInterval(() => {
  if (document.hasFocus() && document.visibilityState === "visible") refresh();
}, REFRESH_EVERY);

document.addEventListener("keydown", (e) => {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (e.key === "Escape") {
    if (!$("nt-palette").hidden) return closePalette();
    if (!$("nt-menu").hidden || !$("nt-transcribe-menu").hidden) return setMenus(null);
    if (!$("nt-item-menu").hidden) return hideItemMenu();
    if (!$("nt-rename").hidden) return endRename();
    if (review) {
      closeReview();
      return focusNote();
    }
    if (!$("nt-history").hidden) return showHistory(false);
    if (!$("nt-askpanel").hidden && $("nt-askpanel").contains(document.activeElement)) return closeAsk();
  }
  if (!mod || e.altKey) return;
  const key = e.key.toLowerCase();
  const run = {
    k: openPalette,
    f: openPalette,
    j: () => ($("nt-askpanel").hidden ? openAsk() : closeAsk()),
    n: newNote,
    t: openDaily,
    e: () => toggleList(),
    o: pickFile,
    l: askForLink,
  }[key];
  if (key === "e" && e.shiftKey) {
    e.preventDefault();
    toggleNav();
  } else if (run && !e.shiftKey) {
    e.preventDefault();
    run();
  } else if (e.key === "Enter" && review) {
    e.preventDefault();
    keepRewrite();
  }
});

for (const b of document.querySelectorAll(".nt-nav-group [data-view]")) {
  b.addEventListener("click", () => showView({ kind: b.dataset.view, value: "" }));
}
$("nt-new-folder").addEventListener("click", () => newFolder(""));
$("nt-daily").addEventListener("click", openDaily);
$("nt-nav-toggle").addEventListener("click", () => toggleNav());
dropTarget($("nt-folders-head"), "");
$("nt-toggle").addEventListener("click", () => toggleList());
$("nt-new").addEventListener("click", newNote);
$("nt-record").addEventListener("click", toggleMeeting);
$("nt-pin").addEventListener("click", togglePin);
$("nt-versions").addEventListener("click", () => showHistory(true));
$("nt-title").addEventListener("click", startRename);
$("nt-copy").addEventListener("click", copyNote);
$("nt-send").addEventListener("click", sendNote);
$("nt-rename").addEventListener("submit", finishRename);
$("nt-rename-cancel").addEventListener("click", endRename);
$("nt-history-back").addEventListener("click", () => showHistory(false));
$("nt-folder").addEventListener("click", () => call("notes_reveal", { path: null }));
$("nt-reveal").textContent = isMac ? "Show in Finder" : "Show in File Explorer";
$("nt-delete").textContent = `Move to ${TRASH}`;
for (const kbd of document.querySelectorAll("kbd[data-keys]")) {
  kbd.textContent = keysLabel(kbd.dataset.keys).textContent;
}

// You used another app while Notes stayed open: Paste into goes there now.
listen("notes-send-target", (event) => {
  sendTarget = event.payload ?? null;
  updateHeader();
});

/**
 * What was asked for once the window is up: "new" or "daily" (voice
 * actions), or "note:<path>" (a splashdown://open?note= link).
 */
function then(what) {
  if (what === "new") newNote();
  else if (what === "daily") openDaily();
  else if (what?.startsWith("note:")) openNote(what.slice(5));
}

listen("notes-then", (event) => then(event.payload));

// Shown again (⌃⌥N or the menu bar), maybe from a different app.
listen("notes-shown", (event) => {
  sendTarget = event.payload ?? null;
  refresh();
  loadTransforms();
  focusNote();
});

function applyView(view) {
  invoke("notes_folders").then((f) => {
    folders = f ?? [];
    renderNav();
  }).catch(() => {});
  sendTarget = view.target ?? null;
  onTop = !!view.on_top;
  $("nt-on-top").setAttribute("aria-checked", String(onTop));
  notes = view.notes;
  showFolder(view.folder);
  if (view.open) {
    showNote(view.open);
  } else {
    current = { path: null, modified: null, source: "" };
    setBody("");
    renderList();
    updateHeader();
    focusNote();
  }
}

/** The dictation key's name, for the hints. */
async function showKey() {
  try {
    const snapshot = await invoke("get_state");
    if (snapshot) showDictationKey(snapshot);
  } catch {
    // The hints say "your dictation key".
  }
}

(async () => {
  try {
    if (localStorage.getItem("notes.list") === "hidden") toggleList(false);
    if (localStorage.getItem("notes.nav") === "hidden" || matchMedia("(max-width: 900px)").matches) toggleNav(false);
    const saved = JSON.parse(localStorage.getItem("notes.view") ?? "null");
    if (saved?.kind) view = saved;
  } catch {
    // Shown by default.
  }
  try {
    applyView(await invoke("notes_load"));
  } catch (e) {
    toast(String(e), true);
  }
  loadTransforms();
  showKey();
  // Opened by a voice action ("new note", "today's note").
  then(new URLSearchParams(location.search).get("then"));
  try {
    const running = await invoke("notes_transcribing");
    if (running) showJob(running);
    const recording = await invoke("meeting_status");
    if (recording) showMeeting(recording);
  } catch {
    // Nothing running.
  }
})();
