"use strict";

// The Notes window: Markdown files in the notes folder (spec §9e), listed
// on the left and edited on the right. A new note becomes a file once it
// has text; after that it's saved as you type.

/** How long typing pauses before the note is saved (ms). */
const SAVE_DELAY = 500;
/** How often the list is re-read while the window has focus, so changes
 *  made in other apps (Obsidian, a sync service) show up (ms). */
const REFRESH_EVERY = 3000;

const isMac = navigator.platform.startsWith("Mac");
const note = $("nt-note");

/** The note in the editor: its file (null until it has text) and the
 *  file's time when loaded, so changes made elsewhere are noticed. */
let current = { path: null, modified: null };
/** The text last saved, so unchanged notes aren't re-saved. */
let savedText = "";
let saveTimer = null;
/** Saves run one at a time, so a new note's file is only created once. */
let saving = Promise.resolve();
let notes = [];
let folder = null;
/** The app "Send to" pastes into, or null. */
let sendTarget = null;

// ---------------------------------------------------------------- saving

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveNow, SAVE_DELAY);
  updateHeader();
}

function saveNow() {
  clearTimeout(saveTimer);
  saveTimer = null;
  saving = saving.then(async () => {
    const text = note.value;
    if (text === savedText) return;
    if (!current.path && text.trim() === "") return;
    try {
      const saved = await invoke("notes_save", { path: current.path, text, base: current.modified });
      savedText = text;
      if (!saved) return;
      const created = !current.path;
      current = { path: saved.note.path, modified: saved.note.modified };
      if (saved.conflict) {
        showNotice("This note was changed in another app. Your version is saved; the other one is in History.");
      }
      if (created) await refresh();
      else updateListEntry(text);
      updateHeader();
    } catch (e) {
      toast(String(e), true);
    }
  });
  return saving;
}

// ---------------------------------------------------------------- the list

/** Lowercase without accents, for search. */
function fold(s) {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

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

function renderList() {
  const query = fold($("nt-search").value.trim());
  const shown = query
    ? notes.filter((n) => fold(`${n.title}\n${n.text}`).includes(query))
    : notes;
  // Pinned first, then the top of the folder, then each subfolder.
  const groups = new Map();
  for (const n of shown) {
    const key = n.pinned ? "\u0000Pinned" : n.folder ? n.folder : "";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(n);
  }
  const keys = [...groups.keys()].sort((a, b) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)));
  const items = [];
  const titled = keys.length > 1;
  for (const key of keys) {
    if (titled) {
      const h = document.createElement("li");
      h.className = "nt-group";
      h.textContent = key === "" ? "Notes" : key.replace("\u0000", "");
      h.setAttribute("aria-hidden", "true");
      items.push(h);
    }
    items.push(...groups.get(key).map(listItem));
  }
  $("nt-list").replaceChildren(...items);
  const empty = $("nt-list-empty");
  empty.hidden = shown.length > 0;
  empty.textContent = query
    ? "No notes match."
    : `No notes yet. Each one is saved as a file in ${folder?.label ?? "your notes folder"}.`;
}

function listItem(n) {
  const li = document.createElement("li");
  const b = document.createElement("button");
  b.className = "nt-item";
  b.setAttribute("aria-current", String(n.path === current.path));
  const title = document.createElement("span");
  title.className = "nt-item-title";
  title.textContent = n.title || "Untitled";
  const meta = document.createElement("span");
  meta.className = "nt-item-meta";
  const firstLine = n.text.split("\n").map((l) => l.trim()).filter(Boolean)[1] ?? "";
  meta.textContent = `${when(n.modified)}  ${firstLine}`;
  b.append(title, meta);
  b.addEventListener("click", () => openNote(n.path));
  li.append(b);
  return li;
}

/** Keeps the list's entry for the open note in step with the editor. */
function updateListEntry(text) {
  const entry = notes.find((n) => n.path === current.path);
  if (!entry) return;
  entry.text = text;
  entry.title = titleOf(text);
  entry.modified = current.modified ?? entry.modified;
  renderList();
}

async function refresh() {
  try {
    notes = await invoke("notes_list");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  const entry = notes.find((n) => n.path === current.path);
  // Changed in another app, and nothing typed here since: show theirs.
  if (entry && current.modified && entry.modified > current.modified && note.value === savedText) {
    try {
      showNote(await invoke("notes_open", { path: current.path }), { keepFocus: true });
    } catch (e) {
      toast(String(e), true);
    }
  }
  renderList();
  updateHeader();
}

// ---------------------------------------------------------------- the note

function titleOf(text) {
  for (const line of text.split("\n")) {
    const t = line.trim().replace(/^#+\s*/, "").trim();
    if (t) return t;
  }
  return "";
}

function showNote(n, { keepFocus = false } = {}) {
  current = { path: n.path, modified: n.modified };
  note.value = n.body;
  savedText = n.body;
  showHistory(false);
  hideNotice();
  renderList();
  updateHeader();
  if (!keepFocus) focusNote();
}

async function openNote(path) {
  if (path === current.path) return focusNote();
  await saveNow();
  try {
    showNote(await invoke("notes_open", { path }));
  } catch (e) {
    toast(String(e), true);
    await refresh();
  }
}

async function newNote() {
  await saveNow();
  current = { path: null, modified: null };
  note.value = "";
  savedText = "";
  showHistory(false);
  hideNotice();
  renderList();
  updateHeader();
  focusNote();
  invoke("notes_new").catch(() => {});
}

function updateHeader() {
  const open = notes.find((n) => n.path === current.path);
  const text = note.value;
  $("nt-title").textContent = titleOf(text) || (current.path ? "Untitled" : "New note");
  const pin = $("nt-pin");
  pin.disabled = !current.path;
  pin.setAttribute("aria-pressed", String(!!open?.pinned));
  pin.textContent = open?.pinned ? "Unpin" : "Pin";
  for (const act of ["rename", "history", "delete"]) {
    document.querySelector(`[data-act="${act}"]`).disabled = !current.path;
  }
  $("nt-status").textContent = current.path ?? "";
  $("nt-status").title = current.path ? `${folder?.path ?? ""}/${current.path}` : "";
  const empty = text.trim() === "";
  $("nt-copy").disabled = empty;
  const send = $("nt-send");
  send.textContent = sendTarget ? `Send to ${sendTarget}` : "Send";
  send.disabled = empty || !sendTarget;
  send.title = sendTarget
    ? `Paste the note into ${sendTarget} (${isMac ? "⌘" : "Ctrl+"}Return)`
    : "Open Notes from another app to send the note there";
}

function focusNote() {
  if (!$("nt-editor").hidden) note.focus();
}

function showNotice(text) {
  const n = $("nt-notice");
  n.textContent = text;
  n.hidden = false;
}

function hideNotice() {
  $("nt-notice").hidden = true;
  if (folder?.problem) showNotice(folder.problem);
}

function showFolder(f) {
  folder = f;
  const b = $("nt-folder");
  b.textContent = f.label;
  b.title = f.synced
    ? `${f.path}\nAlso kept by ${f.synced}. Click to show the folder.`
    : `${f.path}\nClick to show the folder.`;
  hideNotice();
}

// ---------------------------------------------------------------- actions

async function copyNote() {
  try {
    await invoke("copy_text", { text: note.value });
    toast("Copied");
  } catch (e) {
    toast(String(e), true);
  }
}

async function sendNote() {
  if ($("nt-send").disabled) return;
  await saveNow();
  try {
    // Hides this window, switches to the app and pastes. On failure the
    // window comes back with the note on the clipboard.
    await invoke("notes_send", { text: note.value });
  } catch (e) {
    toast(String(e), true);
  }
}

async function togglePin() {
  if (!current.path) return;
  const open = notes.find((n) => n.path === current.path);
  try {
    await invoke("notes_pin", { path: current.path, pinned: !open?.pinned });
    await refresh();
  } catch (e) {
    toast(String(e), true);
  }
}

function startRename() {
  if (!current.path) return;
  const name = current.path.split("/").pop().replace(/\.md$/i, "");
  $("nt-title").hidden = true;
  $("nt-rename").hidden = false;
  const input = $("nt-rename-input");
  input.value = name;
  input.focus();
  input.select();
}

function endRename() {
  $("nt-rename").hidden = true;
  $("nt-title").hidden = false;
}

async function finishRename(e) {
  e.preventDefault();
  await saveNow();
  try {
    const path = await invoke("notes_rename", { path: current.path, name: $("nt-rename-input").value });
    current.path = path;
    endRename();
    await refresh();
    focusNote();
  } catch (err) {
    toast(String(err), true);
  }
}

async function deleteNote() {
  if (!current.path) return;
  const ok = await askConfirm("Move this note to the Trash?", {
    action: "Move to Trash",
    detail: isMac ? "You can get it back from the Trash in Finder." : "You can get it back from the Recycle Bin.",
  });
  if (!ok) return;
  await saveNow();
  try {
    await invoke("notes_delete", { path: current.path });
    toast(isMac ? "Moved to the Trash" : "Moved to the Recycle Bin");
    await refresh();
    const next = notes[0];
    if (next) showNote(await invoke("notes_open", { path: next.path }));
    else newNote();
  } catch (e) {
    toast(String(e), true);
  }
}

async function changeFolder() {
  await saveNow();
  let picked;
  try {
    picked = await invoke("notes_pick_folder");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  if (!picked || picked === folder?.path) return;
  const count = notes.length;
  const moveToo = count > 0 && await askConfirm(`Move your ${count === 1 ? "note" : `${count} notes`} there too?`, {
    action: "Move",
    detail: "Otherwise they stay where they are, and Notes shows the new folder's notes.",
  });
  try {
    const view = await invoke("notes_set_folder", { folder: picked, moveNotesToo: moveToo });
    applyView(view);
    toast(`Notes are now in ${view.folder.label}`);
  } catch (e) {
    toast(String(e), true);
  }
}

async function showHistory(show) {
  $("nt-editor").hidden = show;
  $("nt-history").hidden = !show;
  if (!show) return focusNote();
  await saveNow();
  let versions = [];
  try {
    versions = await invoke("notes_versions", { path: current.path });
  } catch (e) {
    toast(String(e), true);
  }
  $("nt-versions").replaceChildren(...versions.map(versionRow));
  $("nt-versions-empty").hidden = versions.length > 0;
  $("nt-history-back").focus();
}

function versionRow(v) {
  const li = document.createElement("li");
  const preview = document.createElement("div");
  preview.className = "nt-preview";
  preview.textContent = v.preview;
  const meta = document.createElement("div");
  meta.className = "nt-meta";
  const time = document.createElement("time");
  time.dateTime = new Date(v.at).toISOString();
  const d = new Date(v.at);
  time.textContent = `${when(v.at)}${d.toDateString() === new Date().toDateString() ? "" : `, ${d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`}`;
  const restore = button("Restore", async () => {
    try {
      showNote(await invoke("notes_restore", { path: current.path, at: v.at }));
      toast("Restored. The version it replaced is in History.");
      await refresh();
    } catch (e) {
      toast(String(e), true);
    }
  }, "link");
  restore.setAttribute("aria-label", `Restore the version from ${time.textContent}`);
  meta.append(time, restore);
  li.append(preview, meta);
  return li;
}

// ---------------------------------------------------------------- menu

function setMenu(open) {
  $("nt-menu").hidden = !open;
  $("nt-more").setAttribute("aria-expanded", String(open));
  if (open) $("nt-menu").querySelector("button:not(:disabled)")?.focus();
}

$("nt-more").addEventListener("click", (e) => {
  e.stopPropagation();
  setMenu($("nt-menu").hidden);
});
document.addEventListener("click", () => setMenu(false));
$("nt-menu").addEventListener("click", (e) => {
  const act = e.target.closest("button")?.dataset.act;
  if (!act) return;
  setMenu(false);
  ({
    rename: startRename,
    history: () => showHistory(true),
    reveal: () => invoke("notes_reveal", { path: current.path }).catch((err) => toast(String(err), true)),
    folder: changeFolder,
    delete: deleteNote,
  })[act]();
});

// ---------------------------------------------------------------- wiring

function toggleList(show = $("nt-side").hidden) {
  $("nt-side").hidden = !show;
  $("nt-toggle").setAttribute("aria-expanded", String(show));
  try {
    localStorage.setItem("notes.list", show ? "shown" : "hidden");
  } catch {
    // Not remembered; fine.
  }
}

note.addEventListener("input", scheduleSave);
// Hiding the window (⌃⌥N, Send, close) takes focus away: save first.
window.addEventListener("blur", saveNow);
document.addEventListener("visibilitychange", saveNow);
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
    if (!$("nt-menu").hidden) return setMenu(false);
    if (!$("nt-rename").hidden) return endRename();
    if (!$("nt-history").hidden) return showHistory(false);
    if (document.activeElement === $("nt-search") && $("nt-search").value) {
      $("nt-search").value = "";
      return renderList();
    }
  }
  if (!mod) return;
  const key = e.key.toLowerCase();
  if (key === "n") {
    e.preventDefault();
    newNote();
  } else if (key === "e") {
    e.preventDefault();
    toggleList();
  } else if (key === "f") {
    e.preventDefault();
    toggleList(true);
    $("nt-search").focus();
  } else if (e.key === "Enter" && document.activeElement === note) {
    e.preventDefault();
    sendNote();
  }
});

$("nt-search").addEventListener("input", renderList);
$("nt-toggle").addEventListener("click", () => toggleList());
$("nt-new").addEventListener("click", newNote);
$("nt-pin").addEventListener("click", togglePin);
$("nt-copy").addEventListener("click", copyNote);
$("nt-send").addEventListener("click", sendNote);
$("nt-rename").addEventListener("submit", finishRename);
$("nt-rename-cancel").addEventListener("click", endRename);
$("nt-history-back").addEventListener("click", () => showHistory(false));
$("nt-folder").addEventListener("click", () => invoke("notes_reveal", { path: null }).catch((e) => toast(String(e), true)));
$("nt-reveal").textContent = isMac ? "Show in Finder" : "Show in File Explorer";

// Shown again (⌃⌥N or the menu bar), maybe from a different app.
listen("notes-shown", (event) => {
  sendTarget = event.payload ?? null;
  refresh();
  focusNote();
});

function applyView(view) {
  sendTarget = view.target ?? null;
  notes = view.notes;
  showFolder(view.folder);
  if (view.open) {
    showNote(view.open);
  } else {
    current = { path: null, modified: null };
    note.value = "";
    savedText = "";
    renderList();
    updateHeader();
    focusNote();
  }
}

(async () => {
  try {
    if (localStorage.getItem("notes.list") === "hidden") toggleList(false);
  } catch {
    // Shown by default.
  }
  try {
    applyView(await invoke("notes_load"));
  } catch (e) {
    toast(String(e), true);
  }
})();

// ---------------------------------------------------------------- preview

/** Sample backend for opening this page as a plain file:// page (see
 *  invoke in common.js). */
async function mockInvoke(cmd, args) {
  const hour = 3600 * 1000;
  const list = [
    { path: "2026-10-31 Launch post.md", title: "Launch post", text: "Launch post\n- lead with the hold-to-talk demo\n- mention it's free and open source", modified: Date.now() - 0.2 * hour, pinned: true, folder: "" },
    { path: "2026-10-31 Call the dentist.md", title: "Call the dentist", text: "Call the dentist\nabout moving Thursday's appointment.", modified: Date.now() - 2 * hour, pinned: false, folder: "" },
    { path: "2026-10-30 Groceries.md", title: "Groceries", text: "Groceries\noat milk, lemons, coffee beans, rice", modified: Date.now() - 30 * hour, pinned: false, folder: "" },
    { path: "Work/Standup.md", title: "Standup", text: "Standup\nWindows build compiles; testing Monday.", modified: Date.now() - 50 * hour, pinned: false, folder: "Work" },
  ];
  const open = (path) => {
    const n = list.find((x) => x.path === path) ?? list[0];
    return { path: n.path, body: n.text, modified: n.modified };
  };
  switch (cmd) {
    case "notes_load":
      return {
        folder: { path: "/Users/you/Documents/Splashdown Notes", label: "Documents › Splashdown Notes", synced: null, problem: null },
        notes: list,
        open: open(list[0].path),
        target: "Mail",
      };
    case "notes_list":
      return list;
    case "notes_open":
      return open(args.path);
    case "notes_versions":
      return [{ at: Date.now() - hour, preview: "Launch post\n- lead with the demo" }];
    default:
      return null;
  }
}
