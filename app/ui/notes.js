"use strict";

// The Notes window: Markdown files in the notes folder, listed
// on the left and edited on the right. A new note becomes a file once it
// has text; after that it's saved as you type. A meeting's note shows as
// tabs (its summary, transcript and the user's own notes); while it
// records, the transcript grows beside a place to write. ⌘K finds any
// note or action, ⌘J asks the notes a question, and a rewrite shows what
// it changed before it's kept.

/** How long typing pauses before the note is saved (ms). */
const SAVE_DELAY = 500;
/** How long typing beside a meeting pauses before it's written (ms). */
const LIVE_NOTES_DELAY = 700;
/** How often the list is re-read while the window has focus, so changes
 *  made in other apps (Obsidian, a sync service) show up (ms). */
const REFRESH_EVERY = 3000;
/** Earlier questions a follow-up in Ask carries along. */
const ASK_EARLIER = 6;
/** Playback speeds the speed button goes through. */
const SPEEDS = [1, 1.25, 1.5, 2];

const isMac = navigator.platform.startsWith("Mac");
const isWindows = navigator.platform.startsWith("Win");
const MY_NOTES = "My notes";

/** The note open: its file (null until it has text), the file's time when
 *  loaded (so changes made elsewhere are noticed) and how it was made. */
let current = { path: null, modified: null, source: "" };
/** The note's whole text, as saved to its file. */
let body = "";
/** The text last saved, so unchanged notes aren't re-saved. */
let savedText = "";
let saveTimer = null;
/** Saves run one at a time, so a new note's file is only created once. */
let saving = Promise.resolve();
let notes = [];
let folder = null;
/** The app "Paste into" pastes into, or null. */
let sendTarget = null;
/** What the list shows: a view of the library ("all", "pinned",
 *  "meetings", "imported"), a folder with the folders in it, or a tag
 *  with the tags under it. */
let view = { kind: "all", value: "" };
/** The notes folder's subfolders (`Work`, `Work/Acme`). */
let folders = [];
/** Where a new note goes: the folder being looked at, and the tag being
 *  looked at, which it's given. */
let newIn = { folder: "", tag: null };
/** The window stays above other apps' windows. */
let onTop = false;
/** A meeting note's tab: summary, transcript, notes or whole. */
let tab = "summary";
/** The meeting note's part the editor shows ({ m, key }), or null when it
 *  shows the whole note. Typing changes that part, nothing else. */
let editing = null;
/** A transcript turn: `**[0:04] You:**` (a meeting) or `**[0:04]**` (a
 *  file). Not just any line starting `**[`, which the user may type. */
const TURN = /^\*\*\[(\d+:\d{2}(?::\d{2})?)\](?: ([^*]+?):)?\*\*\s?(.*)$/;

const editor = NoteEditor.create($("nt-edit"), {
  label: "Note",
  placeholder: "",
  onChange: edited,
  onSend: sendNote,
  onPaste: pasted,
  onTag: (tag) => {
    toggleNav(true);
    showView({ kind: "tag", value: tag });
  },
});
const liveEditor = NoteEditor.create($("nt-live-notes"), {
  label: "My notes",
  placeholder: "What matters to you in this meeting",
  onChange: liveNotesEdited,
  onSend: () => {},
});

// ---------------------------------------------------------------- saving

function edited() {
  if (editing) {
    setPart(editing.m, editing.key, editor.text);
    body = composeMeeting(editing.m);
  } else {
    body = editor.text;
  }
  scheduleSave();
  updateStart();
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveNow, SAVE_DELAY);
  updateHeader();
}

function saveNow() {
  clearTimeout(saveTimer);
  saveTimer = null;
  saving = saving.then(async () => {
    const text = body;
    if (text === savedText) return;
    if (!current.path && text.trim() === "") return;
    try {
      const created = !current.path;
      const saved = await invoke("notes_save", {
        path: current.path,
        text,
        base: current.modified,
        folder: created ? newIn.folder : null,
      });
      savedText = text;
      if (!saved) return;
      if (created && newIn.tag) {
        const tagged = await invoke("notes_set_tags", { path: saved.note.path, tags: [newIn.tag] }).catch(() => null);
        if (tagged) saved.note = tagged;
      }
      current = {
        path: saved.note.path,
        modified: saved.note.modified,
        source: saved.note.source ?? current.source,
        frontTags: saved.note.front_tags ?? current.frontTags,
      };
      if (saved.conflict) {
        showNotice("This note was changed in another app. Your version is saved; the other one is in Versions.");
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

/** "meeting", "audio" (an app's), "file", "link", "capture" or "note". */
function kindOf(source, text) {
  if (source === "meeting") return "meeting";
  if (source === "app-audio") return "audio";
  if (source === "recording") return "recording";
  if (source === "file") return /Transcribed from <https?:/.test(text) ? "link" : "file";
  if (source === "capture") return "capture";
  return "note";
}

const KIND_ICON = { meeting: "people", audio: "sound", recording: "mic", file: "file", link: "link", capture: "mic", note: "note" };
const KIND_LABEL = {
  meeting: "Meeting",
  audio: "Transcribed from an app",
  recording: "Recorded from the microphone",
  file: "Transcribed from a file",
  link: "Transcribed from a link",
  capture: "Dictated",
  note: "Note",
};

function icon(name, cls = "ic") {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", cls);
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

/** The list's heading for a note: Pinned, Today, Yesterday, Previous 7
 *  days, then by month. */
function groupOf(n) {
  if (n.pinned) return "Pinned";
  const d = new Date(n.modified);
  const now = new Date();
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((day(now) - day(d)) / 86400000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return "Previous 7 days";
  const opts = { month: "long" };
  if (d.getFullYear() !== now.getFullYear()) opts.year = "numeric";
  return d.toLocaleDateString([], opts);
}

/** A line of the note under its title, without Markdown marks. */
function snippetOf(text) {
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  const line = lines[1] ?? "";
  return line
    .replace(/^#+\s*/, "")
    .replace(/^[-*+]\s+(\[[ xX]\]\s+)?/, "")
    .replace(/\*\*|__|`/g, "")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1");
}

/** Whether `path` is `folder` or inside it. */
function within(path, folder) {
  return path === folder || path.startsWith(`${folder}/`);
}

/** Whether note `n` has tag `tag` or one under it (any case). */
function hasTag(n, tag) {
  const t = tag.toLowerCase();
  return (n.tags ?? []).some((x) => within(x.toLowerCase(), t));
}

function shownInList(n, v = view) {
  const kind = kindOf(n.source, n.text);
  switch (v.kind) {
    case "pinned":
      return n.pinned;
    case "meetings":
      return kind === "meeting" || kind === "recording";
    case "imported":
      return kind === "file" || kind === "link" || kind === "audio";
    case "folder":
      return within(n.folder, v.value);
    case "tag":
      return hasTag(n, v.value);
    case "todo":
      return /^\s*[-*+] \[ \]/m.test(n.text);
    case "today":
      return new Date(n.modified).toDateString() === new Date().toDateString();
    case "untagged":
      return !n.tags?.length;
    default:
      return true;
  }
}

const VIEW_NAMES = {
  all: "All notes",
  pinned: "Pinned",
  todo: "To do",
  today: "Changed today",
  meetings: "Meetings & recordings",
  imported: "Transcripts",
  untagged: "Untagged",
};

function viewName(v = view) {
  if (v.kind === "folder") return v.value.split("/").pop();
  if (v.kind === "tag") return `#${v.value}`;
  return VIEW_NAMES[v.kind] ?? "All notes";
}

function renderList() {
  const shown = notes.filter((n) => shownInList(n));
  const items = [];
  let last = null;
  for (const n of shown) {
    const group = groupOf(n);
    if (group !== last) {
      const h = document.createElement("li");
      h.className = "nt-group";
      h.textContent = group;
      h.setAttribute("aria-hidden", "true");
      items.push(h);
      last = group;
    }
    items.push(listItem(n));
  }
  $("nt-list").replaceChildren(...items);
  const empty = $("nt-list-empty");
  empty.hidden = shown.length > 0;
  empty.textContent = notes.length === 0
    ? `No notes yet. Each one is saved as a file in ${folder?.label ?? "your notes folder"}.`
    : {
      pinned: "No pinned notes. Pin one from its ⋯ menu.",
      meetings: "No meetings or recordings yet.",
      imported: "Nothing transcribed yet.",
      folder: "No notes in this folder. Drag one here, or press + to start one.",
      todo: "Nothing to do. A note with an unticked - [ ] checkbox shows here.",
      today: "No notes changed today.",
      untagged: "Every note has a tag.",
      tag: "No notes with this tag.",
    }[view.kind] ?? "";
  $("nt-folder-label").textContent = folder?.label ?? "";
  $("nt-view-name").textContent = viewName();
  $("nt-view-count").textContent = shown.length ? String(shown.length) : "";
  $("nt-new").title = view.kind === "folder" ? `New note in ${viewName()}` : view.kind === "tag" ? `New note tagged ${viewName()}` : "New note";
  renderNav();
}

function listItem(n) {
  const li = document.createElement("li");
  const b = document.createElement("button");
  b.className = "nt-item";
  b.setAttribute("aria-current", String(n.path === current.path));
  const kind = kindOf(n.source, n.text);
  const text = document.createElement("span");
  text.className = "nt-item-text";
  const title = document.createElement("span");
  title.className = "nt-item-title";
  title.textContent = n.title || "Untitled";
  const meta = document.createElement("span");
  meta.className = "nt-item-meta";
  const todo = kind === "meeting" ? (n.text.match(/^\s*- \[ \]/gm) ?? []).length : 0;
  const detail = todo ? `${todo} to do` : snippetOf(n.text);
  // Its folder, unless that's the one being looked at.
  const where = view.kind === "folder"
    ? n.folder.slice(view.value.length + 1)
    : n.folder && n.folder !== "Meetings" ? n.folder : "";
  meta.textContent = [when(n.modified), where, detail].filter(Boolean).join(" · ");
  text.append(title, meta);
  if (n.tags?.length) {
    const tags = document.createElement("span");
    tags.className = "nt-item-meta nt-item-tags";
    tags.textContent = n.tags.slice(0, 3).map((t) => `#${t}`).join(" ");
    text.append(tags);
  }
  b.append(icon(KIND_ICON[kind]), text);
  b.addEventListener("click", () => openNote(n.path));
  b.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    showItemMenu(n, { x: e.clientX, y: e.clientY });
  });
  const more = document.createElement("button");
  more.className = "nt-iconbtn nt-item-more";
  more.setAttribute("aria-label", `Actions for ${n.title || "Untitled"}`);
  more.setAttribute("aria-haspopup", "menu");
  more.title = "Pin, rename, show in Finder or move to the Trash";
  more.append(icon("more"));
  more.addEventListener("click", (e) => {
    e.stopPropagation();
    const r = more.getBoundingClientRect();
    showItemMenu(n, { x: r.left, y: r.bottom + 4 });
  });
  li.className = "nt-item-row";
  if (n.pinned) li.dataset.pinned = "true";
  // Dragged onto a folder, it moves there.
  dragToFolder(b, { note: n.path }, n.title || "Untitled");
  li.append(b, more);
  return li;
}

/** A note's own menu in the list (its ⋯, or a right-click): what the
 *  header does for the open note, for any note. */
/** A menu item for `showMenu`. */
function menuItem(label, run, { disabled = false, danger = false } = {}) {
  const b = document.createElement("button");
  b.setAttribute("role", "menuitem");
  b.textContent = label;
  b.disabled = disabled;
  if (danger) b.className = "danger";
  b.addEventListener("click", () => {
    hideItemMenu();
    run();
  });
  return b;
}

/** Shows a menu of `items` at `at` (kept on screen). */
function showMenu(items, at) {
  setMenus(null);
  const menu = $("nt-item-menu");
  menu.replaceChildren(...items);
  menu.hidden = false;
  const w = menu.offsetWidth;
  const h = menu.offsetHeight;
  menu.style.left = `${Math.max(4, Math.min(at.x, innerWidth - w - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(at.y, innerHeight - h - 4))}px`;
  menu.querySelector("button:not(:disabled)")?.focus();
}

function showItemMenu(n, at) {
  const busy = (!!job && job.note === n.path) || (!!meeting && meeting.note === n.path);
  const item = menuItem;
  // Acting on another note opens it first, as the header acts on the open one.
  const onIt = (run) => async () => {
    if (n.path !== current.path) await openNote(n.path);
    if (current.path === n.path) run();
  };
  showMenu([
    item(n.pinned ? "Unpin" : "Pin to the top", () => setPinned(n.path, !n.pinned)),
    item("Rename…", onIt(startRename), { disabled: busy }),
    item("Move to…", () => moveNoteTo(n.path), { disabled: busy }),
    item(isMac ? "Show in Finder" : "Show in File Explorer", () =>
      invoke("notes_reveal", { path: n.path }).catch((err) => toast(String(err), true)),
    ),
    ...(hasRecording(n.text) ? [item("Transcribe again…", () => transcribeAgain(n.path, at), { disabled: busy })] : []),
    document.createElement("hr"),
    item("Move to Trash", onIt(deleteNote), { disabled: busy, danger: true }),
  ], at);
}

function hideItemMenu() {
  $("nt-item-menu").hidden = true;
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
    [notes, folders] = await Promise.all([invoke("notes_list"), invoke("notes_folders")]);
    folders ??= [];
  } catch (e) {
    toast(String(e), true);
    return;
  }
  const entry = notes.find((n) => n.path === current.path);
  // Changed in another app, and nothing typed here since: show theirs,
  // without moving focus or leaving Versions.
  if (entry && current.modified && entry.modified > current.modified && body === savedText && !review) {
    try {
      const n = await invoke("notes_open", { path: current.path });
      if (n && n.path === current.path && body === savedText) {
        current.modified = n.modified;
        current.frontTags = n.front_tags ?? current.frontTags;
        setBody(n.body, { reload: true });
      }
    } catch (e) {
      toast(String(e), true);
    }
  }
  renderList();
  updateHeader();
}

// ---------------------------------------------------------------- folders and tags

/** Folders and tags folded shut, as "folder:work" or "tag:acme". */
const folded = new Set(loadList("notes.folded"));

function loadList(key) {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "[]");
  } catch {
    return [];
  }
}

function saveList(key, items) {
  try {
    localStorage.setItem(key, JSON.stringify([...items]));
  } catch {
    // Not remembered; fine.
  }
}

/** Shows view `v` in the list, and remembers it. */
function showView(v) {
  view = v;
  try {
    localStorage.setItem("notes.view", JSON.stringify(v));
  } catch {
    // Not remembered; fine.
  }
  renderList();
  // A narrow window has folders and tags over the list: out of the way.
  if (matchMedia("(max-width: 900px)").matches) toggleNav(false);
}

/** Opens today's daily note, making it if it isn't there yet. */
async function openDaily() {
  await saveNow();
  await flushLiveNotes();
  try {
    showNote(await invoke("notes_daily"));
    await refresh();
  } catch (e) {
    toast(String(e), true);
  }
}

function toggleNav(show = $("nt-nav").hidden) {
  $("nt-nav").hidden = !show;
  $("nt-nav-toggle").setAttribute("aria-expanded", String(show));
  try {
    localStorage.setItem("notes.nav", show ? "shown" : "hidden");
  } catch {
    // Not remembered; fine.
  }
}

function parentOf(path) {
  return path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
}

function renderNav() {
  for (const b of document.querySelectorAll(".nt-nav-group [data-view]")) {
    b.setAttribute("aria-current", String(view.kind === b.dataset.view));
  }
  for (const c of document.querySelectorAll(".nt-nav-group [data-count]")) {
    const n = notes.filter((x) => shownInList(x, { kind: c.dataset.count, value: "" })).length;
    c.textContent = n ? String(n) : "";
  }
  $("nt-folders").replaceChildren(...treeRows(folderTree(), "folder"));
  const tags = tagTree();
  $("nt-tags").replaceChildren(...treeRows(tags, "tag"));
  $("nt-tags-empty").hidden = tags.length > 0;
  const known = new Map();
  for (const n of notes) for (const t of n.tags ?? []) known.set(t.toLowerCase(), t);
  $("nt-taglist").replaceChildren(
    ...[...known.values()].sort().map((t) => Object.assign(document.createElement("option"), { value: t })),
  );
}

/** Every folder, with the ones notes are in (which the folder list may
 *  have missed), as a tree. */
function folderTree() {
  const all = new Set(folders);
  for (const n of notes) {
    for (let f = n.folder; f; f = parentOf(f)) all.add(f);
  }
  return buildTree([...all], (path) => notes.filter((n) => within(n.folder, path)).length);
}

/** Every tag, and the tags above nested ones (`work` for `work/acme`). */
function tagTree() {
  const all = new Map();
  for (const n of notes) {
    for (const t of n.tags ?? []) {
      const parts = t.split("/");
      for (let i = 1; i <= parts.length; i++) {
        const p = parts.slice(0, i).join("/");
        if (!all.has(p.toLowerCase())) all.set(p.toLowerCase(), p);
      }
    }
  }
  return buildTree([...all.values()], (path) => notes.filter((n) => hasTag(n, path)).length);
}

/** Paths ("a", "a/b") as a sorted tree of { name, path, count, children }. */
function buildTree(paths, count) {
  const roots = [];
  const byPath = new Map();
  const sorted = [...paths].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true }));
  for (const path of sorted) {
    const node = { name: path.split("/").pop(), path, count: count(path), children: [] };
    byPath.set(path.toLowerCase(), node);
    const parent = byPath.get(parentOf(path).toLowerCase());
    (parent && parentOf(path) ? parent.children : roots).push(node);
  }
  return roots;
}

/** The rows of a folder or tag tree. */
function treeRows(nodes, kind, depth = 0) {
  return nodes.map((node) => {
    const li = document.createElement("li");
    li.setAttribute("role", "treeitem");
    const key = `${kind}:${node.path.toLowerCase()}`;
    const hasChildren = node.children.length > 0;
    if (hasChildren) li.setAttribute("aria-expanded", String(!folded.has(key)));
    const here = view.kind === kind && view.value.toLowerCase() === node.path.toLowerCase();

    const row = document.createElement("div");
    row.className = "nt-nav-row";
    row.style.setProperty("--depth", String(depth));
    row.setAttribute("aria-current", String(here));
    const twisty = document.createElement("button");
    twisty.className = `nt-twisty${hasChildren ? "" : " none"}`;
    twisty.tabIndex = hasChildren ? 0 : -1;
    twisty.setAttribute("aria-label", folded.has(key) ? `Open ${node.name}` : `Close ${node.name}`);
    twisty.append(icon("chev"));
    twisty.addEventListener("click", () => {
      if (folded.has(key)) folded.delete(key);
      else folded.add(key);
      saveList("notes.folded", folded);
      renderNav();
    });
    const main = document.createElement("button");
    main.className = "nt-nav-main";
    if (here) main.setAttribute("aria-current", "true");
    const name = document.createElement("span");
    name.className = "nt-nav-name";
    name.textContent = node.name;
    main.append(icon(kind === "folder" ? "folder" : "tag"), name);
    main.title = kind === "folder" ? node.path : `#${node.path}`;
    main.addEventListener("click", () => showView({ kind, value: node.path }));
    row.append(twisty, main);
    if (kind === "folder") {
      const more = document.createElement("button");
      more.className = "nt-iconbtn nt-tree-more";
      more.setAttribute("aria-label", `Actions for ${node.name}`);
      more.setAttribute("aria-haspopup", "menu");
      more.append(icon("more"));
      more.addEventListener("click", (e) => {
        e.stopPropagation();
        const r = more.getBoundingClientRect();
        folderMenu(node.path, { x: r.left, y: r.bottom + 4 });
      });
      main.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        folderMenu(node.path, { x: e.clientX, y: e.clientY });
      });
      // Dragged onto another folder (or "Folders"), it moves there.
      dragToFolder(main, { folder: node.path }, node.name);
      dropTarget(row, node.path);
      row.append(more);
    }
    const count = document.createElement("span");
    count.className = "nt-count";
    count.textContent = node.count ? String(node.count) : "";
    row.append(count);
    li.append(row);
    if (hasChildren) {
      const ul = document.createElement("ul");
      ul.setAttribute("role", "group");
      ul.append(...treeRows(node.children, kind, depth + 1));
      li.append(ul);
    }
    return li;
  });
}

/** Lets a note or a folder be dropped on `el`, to move it into
 *  `folderPath` ("" for the top). */
function dropTarget(el, folderPath) {
  el.dataset.drop = folderPath;
}

/** Drags `what` ({ note } or { folder }) by `el` to a folder, with the
 *  pointer. The window's own drag and drop is for files from Finder: it
 *  takes every system drag, so this never starts one. */
function dragToFolder(el, what, label) {
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    const start = { x: e.clientX, y: e.clientY };
    let ghost = null;
    let over = null;
    const move = (ev) => {
      if (!ghost) {
        if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < 6) return;
        ghost = document.createElement("div");
        ghost.className = "nt-ghost";
        ghost.textContent = label;
        document.body.append(ghost);
        document.body.classList.add("nt-dragging");
      }
      ghost.style.left = `${ev.clientX + 12}px`;
      ghost.style.top = `${ev.clientY + 10}px`;
      const target = document.elementFromPoint(ev.clientX, ev.clientY)?.closest("[data-drop]") ?? null;
      if (over !== target) {
        over?.classList.remove("nt-drop-here");
        over = target;
        over?.classList.add("nt-drop-here");
      }
    };
    const up = () => {
      removeEventListener("pointermove", move);
      removeEventListener("pointerup", up);
      removeEventListener("pointercancel", up);
      if (!ghost) return;
      ghost.remove();
      document.body.classList.remove("nt-dragging");
      over?.classList.remove("nt-drop-here");
      // The click that ends a drag isn't one.
      const swallow = (c) => {
        c.stopPropagation();
        c.preventDefault();
      };
      addEventListener("click", swallow, { capture: true, once: true });
      setTimeout(() => removeEventListener("click", swallow, { capture: true }), 0);
      if (!over) return;
      const to = over.dataset.drop;
      if (what.note) moveNote(what.note, to);
      else moveFolder(what.folder, to);
    };
    addEventListener("pointermove", move);
    addEventListener("pointerup", up);
    addEventListener("pointercancel", up);
  });
}

function folderMenu(path, at) {
  const busy = (!!meeting && within(meeting.note, path)) || (!!job && within(job.note ?? "", path));
  showMenu([
    menuItem("New note here", async () => {
      showView({ kind: "folder", value: path });
      await newNote();
    }),
    menuItem("New folder inside…", () => newFolder(path)),
    document.createElement("hr"),
    menuItem("Rename…", () => renameFolder(path), { disabled: busy }),
    menuItem("Move to…", () => moveFolderTo(path), { disabled: busy }),
    document.createElement("hr"),
    menuItem("Move to Trash", () => deleteFolder(path), { disabled: busy, danger: true }),
  ], at);
}

async function newFolder(parent) {
  const name = await askText("New folder", {
    detail: parent ? `Inside ${parent.split("/").join(" › ")}.` : "A folder in your notes folder, which other apps see too.",
    placeholder: "Name",
    action: "Create",
  });
  if (!name) return null;
  try {
    const path = await invoke("notes_create_folder", { parent, name });
    folded.delete(`folder:${parent.toLowerCase()}`);
    saveList("notes.folded", folded);
    await refresh();
    showView({ kind: "folder", value: path });
    return path;
  } catch (e) {
    toast(String(e), true);
    return null;
  }
}

/** Keeps the open note and the list's view with a folder that moved. */
function followFolder(from, to) {
  if (current.path && within(current.path, from)) current.path = to + current.path.slice(from.length);
  if (view.kind === "folder" && within(view.value, from)) {
    showView({ kind: "folder", value: to + view.value.slice(from.length) });
  }
}

async function renameFolder(path) {
  const old = path.split("/").pop();
  const name = await askText("Rename the folder", { value: old, action: "Rename" });
  if (!name || name === old) return;
  await saveNow();
  try {
    const moved = await invoke("notes_move_folder", { path, parent: parentOf(path), name });
    followFolder(path, moved);
    await refresh();
  } catch (e) {
    toast(String(e), true);
  }
}

async function moveFolder(path, parent) {
  if (parentOf(path) === parent || within(parent, path)) return;
  await saveNow();
  try {
    const moved = await invoke("notes_move_folder", { path, parent, name: path.split("/").pop() });
    followFolder(path, moved);
    folded.delete(`folder:${parent.toLowerCase()}`);
    await refresh();
    toast(`Moved to ${parent ? parent.split("/").pop() : "the top level"}`);
  } catch (e) {
    toast(String(e), true);
  }
}

async function deleteFolder(path) {
  const name = path.split("/").pop();
  const count = notes.filter((n) => within(n.folder, path)).length;
  const ok = await askConfirm(`Move “${name}” to the Trash?`, {
    action: "Move to Trash",
    detail: `${count ? `The ${count === 1 ? "note" : `${count} notes`} in it go too. ` : ""}${isMac ? "You can get it back from the Trash in Finder." : "You can get it back from the Recycle Bin."}`,
  });
  if (!ok) return;
  await saveNow();
  try {
    await invoke("notes_delete_folder", { path });
    if (view.kind === "folder" && within(view.value, path)) showView({ kind: "all", value: "" });
    const leaving = current.path && within(current.path, path);
    await refresh();
    if (leaving) {
      const next = notes[0];
      if (next) showNote(await invoke("notes_open", { path: next.path }));
      else newNote();
    }
    toast(isMac ? "Moved to the Trash" : "Moved to the Recycle Bin");
  } catch (e) {
    toast(String(e), true);
  }
}

async function moveNote(path, folderPath) {
  const n = notes.find((x) => x.path === path);
  if (n && n.folder === folderPath) return;
  if (path === current.path) await saveNow();
  try {
    const moved = await invoke("notes_move", { path, folder: folderPath });
    if (current.path === path) current.path = moved;
    await refresh();
    toast(`Moved to ${folderPath ? folderPath.split("/").pop() : "the top level"}`);
  } catch (e) {
    toast(String(e), true);
  }
}

/** Asks where to move a note or folder: the folders, as a list. */
function pickFolder(title, { except = null, here = null } = {}) {
  return new Promise((resolve) => {
    const dialog = $("nt-moveto");
    $("nt-moveto-title").textContent = title;
    const rows = [{ path: "", name: "Top level", depth: 0 }];
    const walk = (nodes, depth) => {
      for (const node of nodes) {
        if (except != null && within(node.path, except)) continue;
        rows.push({ path: node.path, name: node.name, depth });
        walk(node.children, depth + 1);
      }
    };
    walk(folderTree(), 1);
    let picked = null;
    let creating = false;
    $("nt-moveto-list").replaceChildren(...rows.map((r) => {
      const b = document.createElement("button");
      b.className = "nt-app";
      b.setAttribute("role", "option");
      b.style.paddingLeft = `${8 + r.depth * 16}px`;
      const ic = document.createElement("span");
      ic.className = "nt-app-ic";
      ic.append(icon(r.path ? "folder" : "inbox"));
      const name = document.createElement("span");
      name.className = "nt-app-name";
      name.textContent = r.name;
      b.append(ic, name);
      if (r.path === here) {
        b.disabled = true;
        b.append(Object.assign(document.createElement("span"), { className: "nt-app-state", textContent: "It's here" }));
      }
      b.addEventListener("click", () => {
        picked = r.path;
        dialog.close();
      });
      return b;
    }));
    $("nt-moveto-new").onclick = async () => {
      creating = true;
      dialog.close();
      resolve(await newFolder(""));
    };
    dialog.onclose = () => {
      dialog.onclose = null;
      if (creating) return;
      resolve(picked);
    };
    dialog.showModal();
    $("nt-moveto-list").querySelector("button:not(:disabled)")?.focus();
  });
}

$("nt-moveto-cancel").addEventListener("click", () => $("nt-moveto").close());

async function moveNoteTo(path) {
  const n = notes.find((x) => x.path === path);
  const to = await pickFolder(`Move “${n?.title || "this note"}” to…`, { here: n?.folder ?? null });
  if (to != null) await moveNote(path, to);
}

async function moveFolderTo(path) {
  const to = await pickFolder(`Move “${path.split("/").pop()}” to…`, { except: path, here: parentOf(path) });
  if (to != null) await moveFolder(path, to);
}

// ---------------------------------------------------------------- a note's tags

/** Shows the open note's tags: the ones in its front matter can be taken
 *  off; the ones in its text are changed by editing it. */
function renderTagBar() {
  const bar = $("nt-tagbar");
  const entry = notes.find((n) => n.path === current.path);
  bar.hidden = !current.path || liveHere() || !$("nt-history").hidden;
  if (bar.hidden) return;
  const front = current.frontTags ?? [];
  const isFront = (t) => front.some((f) => f.toLowerCase() === t.toLowerCase());
  const tags = [...(entry?.tags ?? [])];
  for (const t of front) if (!tags.some((x) => x.toLowerCase() === t.toLowerCase())) tags.push(t);
  $("nt-tagchips").replaceChildren(...tags.map((t) => {
    const chip = document.createElement("span");
    chip.className = `nt-tagchip${isFront(t) ? "" : " inline"}`;
    const name = document.createElement("button");
    name.className = "nt-tagname";
    name.textContent = `#${t}`;
    name.title = isFront(t) ? `Show the notes tagged #${t}` : `In the note's text. Show the notes tagged #${t}`;
    name.addEventListener("click", () => {
      toggleNav(true);
      showView({ kind: "tag", value: t });
    });
    chip.append(name);
    if (isFront(t)) {
      const x = document.createElement("button");
      x.textContent = "×";
      x.setAttribute("aria-label", `Remove the tag ${t}`);
      x.addEventListener("click", () => setTags(front.filter((f) => f.toLowerCase() !== t.toLowerCase())));
      chip.append(x);
    }
    return chip;
  }));
  $("nt-addtag").textContent = "";
  $("nt-addtag").append(icon("tag", "ic sm"), document.createTextNode(tags.length ? "" : "Add a tag"));
  $("nt-addtag").setAttribute("aria-label", "Add a tag");
  $("nt-addtag").title = "Add a tag";
}

async function setTags(tags) {
  if (!current.path) return;
  await saveNow();
  try {
    const note = await invoke("notes_set_tags", { path: current.path, tags });
    current.frontTags = note.front_tags;
    current.modified = note.modified;
    await refresh();
  } catch (e) {
    toast(String(e), true);
  }
}

function startTag() {
  if (!current.path) return toast("Type something first: a note is saved once it has text.");
  $("nt-addtag").hidden = true;
  $("nt-tagform").hidden = false;
  $("nt-taginput").value = "";
  $("nt-taginput").focus();
}

function endTag() {
  $("nt-tagform").hidden = true;
  $("nt-addtag").hidden = false;
}

$("nt-addtag").addEventListener("click", startTag);
$("nt-tagform").addEventListener("submit", async (e) => {
  e.preventDefault();
  const tag = $("nt-taginput").value.trim().replace(/^#+/, "").split(/\s+/).join("-");
  endTag();
  if (!tag) return;
  const front = current.frontTags ?? [];
  if (front.some((f) => f.toLowerCase() === tag.toLowerCase())) return;
  await setTags([...front, tag]);
  focusNote();
});
$("nt-taginput").addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.stopPropagation();
    endTag();
    focusNote();
  }
});
$("nt-taginput").addEventListener("blur", () => setTimeout(endTag, 150));

// ---------------------------------------------------------------- the note

function titleOf(text) {
  for (const line of text.split("\n")) {
    const t = line.trim().replace(/^#+\s*/, "").trim();
    if (t) return t;
  }
  return "";
}

/** Shows `text` as the open note's text. `reload`: the same note, changed
 *  on disk (keeps the cursor and undo); otherwise another note. */
function setBody(text, { reload = false } = {}) {
  body = text;
  savedText = text;
  if (!reload) {
    asMeeting = isMeeting();
    if (asMeeting) tab = parseMeeting(text).parts.some((p) => p.key === "summary") ? "summary" : "transcript";
  }
  render({ reload });
}

function showNote(n, { keepFocus = false } = {}) {
  // A rename in progress was for the note being left.
  endRename();
  closeReview();
  current = { path: n.path, modified: n.modified, source: n.source ?? "", frontTags: n.front_tags ?? [] };
  showHistory(false);
  hideNotice();
  setBody(n.body);
  renderList();
  updateHeader();
  if (askState.scope === "note") renderAsk();
  if (!keepFocus) focusNote();
}

async function openNote(path) {
  if (path === current.path) return focusNote();
  await saveNow();
  await flushLiveNotes();
  try {
    showNote(await invoke("notes_open", { path }));
  } catch (e) {
    toast(String(e), true);
    await refresh();
  }
}

async function newNote() {
  await saveNow();
  await flushLiveNotes();
  closeReview();
  endRename();
  newIn = {
    folder: view.kind === "folder" ? view.value : "",
    tag: view.kind === "tag" ? view.value : null,
  };
  current = { path: null, modified: null, source: "" };
  showHistory(false);
  hideNotice();
  setBody("");
  renderList();
  updateHeader();
  focusNote();
  invoke("notes_new").catch(() => {});
}

/** The open note shows as a meeting's tabs: decided when it's opened, so
 *  a note never turns into one while it's being written. */
let asMeeting = false;

/** A meeting's note (or an app's audio, which shows the same way): made
 *  by recording one, or a transcript of turns. */
function isMeeting() {
  return ["meeting", "app-audio", "recording"].includes(current.source) || /^\*\*\[[\d:]+\] (You|Them):\*\*/m.test(body);
}

/** The meeting recording into the open note, if one is. */
function liveHere() {
  return !!meeting && meeting.note === current.path;
}

/** The note being written to by a transcription or a meeting, so typing
 *  can't be lost under its next paragraph. */
function locked() {
  return (!!job && !job.downloading && job.note === current.path) || liveHere();
}

/** Draws the note: plain, a meeting's tabs, or a meeting recording now. */
function render({ reload = false } = {}) {
  const live = liveHere();
  const meetingNote = live || asMeeting;
  $("nt-live").hidden = !live;
  $("nt-tabs").hidden = live || !meetingNote;
  const m = meetingNote ? parseMeeting(body) : null;
  const part = m && m.parts.find((p) => p.key === tab);
  const turns = m ? turnsOf(body) : null;
  const recording = m ? recordingOf(body) : null;

  // What the editor shows, if anything.
  let edits = null;
  editing = null;
  let emptyText = null;
  if (live) {
    renderTurns($("nt-live-transcript"), turns, { live: true });
  } else if (!meetingNote || tab === "whole") {
    edits = body;
  } else if (tab === "summary") {
    if (part?.text.trim()) {
      edits = part.text;
      editing = { m, key: "summary" };
    }
    else emptyText = turns.some((t) => t.time != null)
      ? "No summary yet. One is written from the transcript and your notes."
      : "No summary yet: there's no transcript to summarize.";
  } else if (tab === "notes") {
    edits = part?.text ?? "";
    editing = { m, key: "notes" };
  }
  if (edits != null) {
    if (reload) editor.reload(edits);
    else editor.load(edits);
  }
  editor.readOnly = locked() || !!review;
  $("nt-edit").setAttribute("aria-busy", String(locked()));
  editor.view.contentDOM.setAttribute("aria-label", meetingNote && tab === "notes" ? "My notes" : meetingNote && tab === "summary" ? "Summary" : "Note");
  $("nt-edit-wrap").hidden = edits == null;
  $("nt-tab-empty").hidden = emptyText == null;
  if (emptyText != null) {
    $("nt-tab-empty-text").textContent = emptyText;
    $("nt-summarize").hidden = !turns.some((t) => t.time != null);
  }
  $("nt-transcript").hidden = live || !meetingNote || tab !== "transcript";
  if (!$("nt-transcript").hidden) {
    renderTurns($("nt-turns"), turns, { recording: !!recording });
    filterTurns();
  }
  for (const b of document.querySelectorAll("#nt-tabs [data-tab]")) {
    b.setAttribute("aria-selected", String(b.dataset.tab === tab));
  }
  $("nt-resummarize").hidden = !(meetingNote && tab === "summary" && part?.text.trim());
  const last = turns ? [...turns].reverse().find((t) => t.time != null) : null;
  $("nt-tab-length").textContent = last ? clock(last.time) : "";
  showPlayer(!live && meetingNote && (tab === "summary" || tab === "transcript") ? recording : null);
  $("nt-copy").parentElement.hidden = live;
  if (live && !reload) liveEditor.load(m.parts.find((p) => p.key === "notes")?.text ?? "");
  updateStart();
  updateHeader();
}

/** The empty note's ways to fill it. */
function updateStart() {
  $("nt-start").hidden = !!body.trim() || asMeeting || $("nt-edit-wrap").hidden || locked();
  $("nt-rewrite").hidden = liveHere() || $("nt-edit-wrap").hidden || editor.readOnly || !editor.text.trim();
}

function updateHeader() {
  const open = notes.find((n) => n.path === current.path);
  const text = body;
  const title = $("nt-title");
  title.textContent = titleOf(text) || (current.path ? "Untitled" : "New note");
  title.disabled = !current.path || locked();
  const kind = kindOf(current.source, text);
  const words = (text.match(/[\p{L}\p{N}]+/gu) ?? []).length;
  $("nt-meta").textContent = current.path
    ? [KIND_LABEL[kind], when(current.modified ?? Date.now()), words ? `${words.toLocaleString()} ${words === 1 ? "word" : "words"}` : ""].filter(Boolean).join(" · ")
    : "Named for you once there's something in it";
  const pin = $("nt-pin");
  pin.disabled = !current.path;
  pin.setAttribute("aria-pressed", String(!!open?.pinned));
  pin.setAttribute("aria-label", open?.pinned ? "Unpin" : "Pin");
  pin.title = open?.pinned ? "Unpin" : "Pin to the top of the list";
  const isLocked = locked();
  $("nt-versions").disabled = !current.path || isLocked;
  for (const act of ["rename", "delete"]) {
    document.querySelector(`#nt-menu [data-act="${act}"]`).disabled = !current.path || isLocked;
  }
  document.querySelector('#nt-menu [data-act="reveal"]').disabled = !current.path;
  // A new folder would split the transcript from the job's.
  document.querySelector('#nt-menu [data-act="folder"]').disabled = !!job || !!meeting;
  $("nt-status").textContent = isLocked && !liveHere()
    ? "Being transcribed. You can edit it when it's done."
    : current.path ?? "";
  $("nt-status").title = current.path ? `${folder?.path ?? ""}/${current.path}` : "";
  $("nt-job-show").hidden = !job || !!job.downloading || isLocked;
  $("nt-meeting").hidden = !meeting || liveHere();
  const record = $("nt-record");
  // While something records, its own bar has the one Stop.
  record.hidden = !!meeting;
  for (const chip of document.querySelectorAll("#nt-rewrite button")) chip.disabled = isLocked || !!review;
  const empty = text.trim() === "";
  $("nt-copy").disabled = empty;
  // Only when there's an app to paste into: the last one you used that
  // takes text (not Finder, not Splashdown).
  const send = $("nt-send");
  send.hidden = !sendTarget;
  send.replaceChildren(document.createTextNode(`Paste into ${sendTarget} `), keysLabel("⌘↩"));
  send.disabled = empty;
  send.title = `Paste the note into ${sendTarget}`;
  renderTagBar();
}

function focusNote() {
  if (!$("nt-history").hidden || !$("nt-review").hidden) return;
  if (liveHere()) return liveEditor.focus();
  if (!$("nt-edit-wrap").hidden) editor.focus();
}

function showNotice(text, action = null) {
  $("nt-notice-text").textContent = text;
  const b = $("nt-notice-act");
  b.hidden = !action;
  if (action) {
    b.textContent = action.label;
    b.onclick = () => {
      hideNotice();
      action.run();
    };
  }
  $("nt-notice").hidden = false;
}

function hideNotice() {
  $("nt-notice").hidden = true;
  if (folder?.problem) showNotice(folder.problem);
}

function showFolder(f) {
  folder = f;
  $("nt-folder").title = f.synced
    ? `${f.path}\nAlso kept by ${f.synced}. Click to show the folder.`
    : `${f.path}\nClick to show the folder.`;
  hideNotice();
}

/** A link pasted into an empty note: offers to transcribe it. */
function pasted(text) {
  const url = text.trim();
  if (body.trim() || !/^https?:\/\/\S+$/.test(url)) return false;
  showNotice("Transcribe what this link plays into a note?", { label: "Transcribe it", run: () => transcribeLink(url) });
  return false;
}

// ---------------------------------------------------------------- meetings: the text

const PART_HEADINGS = { summary: "Summary", notes: MY_NOTES, transcript: "Transcript" };

/** A meeting note's parts: what's before them (title, where it was
 *  recorded) and each `## Summary`, `## My notes` and the transcript (its
 *  `## Transcript` heading, or its first turn), in the order they're in. */
function parseMeeting(text) {
  const head = [];
  const parts = [];
  let part = null;
  for (const line of text.split("\n")) {
    const h = /^## (.+?)\s*$/.exec(line);
    const key = h && Object.keys(PART_HEADINGS).find((k) => PART_HEADINGS[k].toLowerCase() === h[1].toLowerCase());
    if (key) {
      part = { key, heading: line, lines: [] };
      parts.push(part);
      continue;
    }
    if (TURN.test(line) && part?.key !== "transcript") {
      part = { key: "transcript", heading: null, lines: [] };
      parts.push(part);
    }
    (part ? part.lines : head).push(line);
  }
  return {
    head: head.join("\n"),
    parts: parts.map((p) => ({ key: p.key, heading: p.heading, text: p.lines.join("\n").trim() })),
  };
}

function composeMeeting(m) {
  let out = m.head.trimEnd();
  for (const p of m.parts) {
    if (!p.text && p.key !== "transcript") continue;
    const gap = out ? "\n\n" : "";
    out += p.heading == null ? `${gap}${p.text}` : `${gap}${p.heading}\n\n${p.text}`;
  }
  return `${out}\n`;
}

/** Puts `text` in part `key`: a new one goes where the app puts it (your
 *  notes first, the summary before the transcript). */
function setPart(m, key, text) {
  const part = m.parts.find((p) => p.key === key);
  if (part) {
    part.text = text.trim();
    return;
  }
  const added = { key, heading: `## ${PART_HEADINGS[key]}`, text: text.trim() };
  if (key === "notes") {
    m.parts.unshift(added);
  } else {
    const at = m.parts.findIndex((p) => p.key === "transcript");
    if (at < 0) {
      m.parts.push(added);
    } else {
      m.parts.splice(at, 0, added);
      // The transcript gets its heading, as when the app summarizes.
      m.parts[at + 1].heading ??= "## Transcript";
    }
  }
}

/** "1:02:30" or "12:05" in seconds. */
function seconds(stamp) {
  return stamp.split(":").reduce((total, n) => total * 60 + Number(n), 0);
}

/** The transcript's turns: `**[0:04] Them:** Hi` (a meeting) or
 *  `**[0:04]** Hi` (a file), and the lines between them. */
function turnsOf(text) {
  const turns = [];
  for (const line of text.split("\n")) {
    if (!line.trim() || /^\[Recording\]\(/.test(line) || /^#/.test(line)) continue;
    const t = TURN.exec(line);
    if (t) {
      turns.push({ time: seconds(t[1]), stamp: t[1], who: t[2] ?? "", text: t[3] });
    } else if (turns.length) {
      // Notes about the recording ("*Ended at 34:10.*") stand apart.
      const aside = /^\*[^*].*\*$/.test(line.trim());
      if (aside) turns.push({ time: null, who: "", text: line.trim().replace(/^\*|\*$/g, "") });
      else turns[turns.length - 1].text += `\n${line}`;
    }
  }
  return turns;
}

/** The kept recording's path in the notes folder, from `[Recording](…)`. */
function recordingOf(text) {
  const m = /^\[Recording\]\(([^)]+)\)\s*$/m.exec(text);
  if (!m) return null;
  try {
    return decodeURI(m[1]);
  } catch {
    return m[1];
  }
}

function renderTurns(list, turns, { live = false, recording = false } = {}) {
  const atEnd = list.scrollTop + list.clientHeight >= list.scrollHeight - 24;
  const rows = turns.map((t, i) => {
    const row = document.createElement("div");
    row.className = "nt-turn";
    row.dataset.index = String(i);
    if (t.time == null) {
      const aside = document.createElement("div");
      aside.className = "nt-aside-line";
      aside.textContent = t.text;
      row.append(aside);
      return row;
    }
    const ts = document.createElement("button");
    ts.className = "nt-ts";
    ts.textContent = t.stamp;
    ts.disabled = !recording;
    ts.title = recording ? `Play from ${t.stamp}` : "";
    ts.setAttribute("aria-label", recording ? `Play from ${t.stamp}` : t.stamp);
    ts.addEventListener("click", () => playFrom(t.time));
    const words = document.createElement("div");
    if (t.who) {
      // A finished note's speakers can be named: click one.
      const who = document.createElement(live ? "div" : "button");
      who.className = `nt-who ${whoClass(t.who)}`;
      who.textContent = t.who;
      if (!live) {
        who.title = `Name ${t.who}`;
        who.setAttribute("aria-label", `${t.who}: name this speaker`);
        who.addEventListener("click", () => {
          const r = who.getBoundingClientRect();
          nameSpeaker(t.who, { x: r.left, y: r.bottom + 4 });
        });
      }
      words.append(who);
    }
    const said = document.createElement("div");
    said.className = "nt-turn-text";
    said.textContent = t.text;
    words.append(said);
    row.append(ts, words);
    return row;
  });
  if (live && rows.length === 0) {
    const wait = document.createElement("p");
    wait.className = "nt-hint";
    wait.textContent = "Each turn shows here a few seconds after it's said.";
    rows.push(wait);
  }
  list.replaceChildren(...rows);
  if (live && atEnd) list.scrollTop = list.scrollHeight;
}

/** A speaker's color: You, Them, and the others told apart in turn. */
function whoClass(who) {
  if (who === "You") return "you";
  if (who === "Them") return "them";
  const others = [...new Set(turnsOf(body).map((t) => t.who).filter((w) => w && w !== "You" && w !== "Them"))];
  return `other${Math.max(0, others.indexOf(who)) % 4}`;
}

/** The people a meeting's note says were invited ("With Dana, Sam."). */
function invited() {
  const m = /^With (.+)\.$/m.exec(parseMeeting(body).head);
  return m ? m[1].split(/,\s*/).filter(Boolean) : [];
}

/** Names speaker `who` in the whole transcript: from the people invited,
 *  or typed. */
function nameSpeaker(who, at) {
  if (locked()) return;
  const taken = new Set(turnsOf(body).map((t) => t.who));
  const names = invited().filter((n) => !taken.has(n));
  showMenu([
    ...names.map((n) => menuItem(n, () => renameSpeaker(who, n))),
    ...(names.length ? [document.createElement("hr")] : []),
    menuItem("Type a name…", async () => {
      const name = await askText(`Who is ${who}?`, {
        detail: "Every turn they said gets this name.",
        placeholder: "Name",
        action: "Name",
      });
      if (name) renameSpeaker(who, name);
    }),
  ], at);
}

function renameSpeaker(who, name) {
  name = name.replace(/[*:\n\[\]]/g, "").trim();
  if (!name || name === who) return;
  const esc = who.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const turn = new RegExp(`^(\\*\\*\\[[\\d:]+\\] )${esc}(:\\*\\*)`, "gm");
  const m = parseMeeting(body.replace(turn, `$1${name}$2`));
  // "(Speaker 2)" after an action item names them too.
  const summary = m.parts.find((p) => p.key === "summary");
  if (summary) summary.text = summary.text.split(`(${who})`).join(`(${name})`);
  body = composeMeeting(m);
  render({ reload: true });
  scheduleSave();
  toast(`${who} is now ${name}`);
}

/** Find in the transcript: shows the turns that have it, marked. */
function filterTurns() {
  const q = fold($("nt-transcript-find").value.trim());
  const turns = turnsOf(body);
  for (const row of $("nt-turns").children) {
    const t = turns[Number(row.dataset.index)];
    const said = row.querySelector(".nt-turn-text");
    if (!t || !said) continue;
    const hit = !q || fold(`${t.who} ${t.text}`).includes(q);
    row.hidden = !hit;
    said.replaceChildren(...marked(t.text, q ? [q] : []));
  }
}

$("nt-transcript-find").addEventListener("input", filterTurns);

for (const b of document.querySelectorAll("#nt-tabs [data-tab]")) {
  b.addEventListener("click", () => showTab(b.dataset.tab));
}

async function showTab(next) {
  if (tab === next) return;
  await saveNow();
  closeReview();
  tab = next;
  render();
  if (tab !== "transcript") focusNote();
}

/** Writes (or rewrites) the summary from the transcript and the user's
 *  notes, as the app does when a meeting ends. */
/** Whether a note links its kept recording (meetings and recordings do,
 *  when Settings keeps them), so it can be transcribed again. */
function hasRecording(text) {
  return /^\[Recording\]\(.+\.wav\)\s*$/m.test(text ?? "");
}

/** Asks which speech model, then transcribes the note's recording again
 *  in place of its transcript (the progress shows like a file's). */
async function transcribeAgain(path, at) {
  let models;
  try {
    models = await invoke("notes_transcribe_again_models", { path });
  } catch (err) {
    return toast(String(err), true);
  }
  if (!models.length) return toast("This note's recording isn't kept, so it can't be transcribed again.", true);
  const start = async (model) => {
    if (path === current.path) await saveNow();
    invoke("notes_transcribe_again", { path, model }).catch((err) => toast(String(err), true));
  };
  const head = document.createElement("div");
  head.className = "nt-menu-head";
  head.textContent = "Transcribe again with";
  const r = $("nt-title")?.getBoundingClientRect();
  showMenu([head, ...models.map((m) => menuItem(m.label, () => start(m.id)))], at ?? { x: r ? r.left : 80, y: r ? r.bottom + 4 : 80 });
}

async function summarize() {
  if (!asMeeting || locked()) return;
  await saveNow();
  const m = parseMeeting(body);
  const transcript = m.parts.find((p) => p.key === "transcript")?.text ?? "";
  if (!transcript.trim()) return toast("There's no transcript to summarize.");
  const mine = m.parts.find((p) => p.key === "notes")?.text.trim();
  const input = mine ? `My notes, written during the meeting:\n${mine}\n\nTranscript:\n${transcript}` : transcript;
  const path = current.path;
  const before = body;
  for (const b of [$("nt-summarize"), $("nt-resummarize")]) b.disabled = true;
  $("nt-status").textContent = "Summarizing…";
  try {
    const id = current.source === "meeting" ? "meeting-summary" : "summary";
    const out = await invoke("notes_transform", { path, id, text: input, extra: null });
    if (out.unchanged) return toast(`Summary: ${out.unchanged}`);
    if (current.path !== path || body !== before) return toast("The note changed meanwhile, so the summary wasn't added.");
    const fresh = parseMeeting(body);
    setPart(fresh, "summary", out.text);
    body = composeMeeting(fresh);
    tab = "summary";
    render();
    scheduleSave();
    toast("Summarized. The note before is in Versions.");
  } catch (e) {
    toast(String(e), true);
  } finally {
    for (const b of [$("nt-summarize"), $("nt-resummarize")]) b.disabled = false;
    updateHeader();
  }
}

$("nt-summarize").addEventListener("click", summarize);
$("nt-resummarize").addEventListener("click", summarize);

// ---------------------------------------------------------------- meetings: the recording

const audio = $("nt-audio");

function mediaUrl(path) {
  const encoded = path.split("/").map(encodeURIComponent).join("/");
  // Windows serves the app's own schemes as http(s)://<scheme>.localhost.
  return isWindows ? `${location.protocol}//notes-media.localhost/${encoded}` : `notes-media://localhost/${encoded}`;
}

/** A recording that wouldn't play, so its player stays hidden. */
let brokenRecording = null;

function showPlayer(path) {
  $("nt-player").hidden = !path || path === brokenRecording;
  const url = path ? mediaUrl(path) : "";
  if (audio.dataset.path === (path ?? "")) return;
  audio.pause();
  audio.dataset.path = path ?? "";
  if (url) audio.src = url;
  else audio.removeAttribute("src");
  $("nt-seek").value = "0";
  $("nt-play-at").textContent = "0:00";
  $("nt-play-length").textContent = "0:00";
  updatePlay();
}

function updatePlay() {
  const playing = !audio.paused;
  $("nt-play").querySelector("use").setAttribute("href", playing ? "#i-pause" : "#i-play");
  $("nt-play").setAttribute("aria-label", playing ? "Pause" : "Play the recording");
}

function playFrom(secs) {
  if (!audio.src) return;
  audio.currentTime = secs;
  audio.play().catch((e) => toast(`Couldn't play the recording: ${e.message}`, true));
}

$("nt-play").addEventListener("click", () => {
  if (audio.paused) audio.play().catch((e) => toast(`Couldn't play the recording: ${e.message}`, true));
  else audio.pause();
});
audio.addEventListener("play", updatePlay);
audio.addEventListener("pause", updatePlay);
audio.addEventListener("loadedmetadata", () => {
  $("nt-seek").max = String(audio.duration || 1);
  $("nt-play-length").textContent = clock(audio.duration || 0);
});
audio.addEventListener("timeupdate", () => {
  $("nt-seek").value = String(audio.currentTime);
  $("nt-play-at").textContent = clock(audio.currentTime);
  // The turn being played.
  const turns = turnsOf(body);
  let now = -1;
  turns.forEach((t, i) => {
    if (t.time != null && t.time <= audio.currentTime) now = i;
  });
  for (const row of $("nt-turns").children) {
    row.classList.toggle("nt-now", !audio.paused && Number(row.dataset.index) === now);
  }
});
audio.addEventListener("error", () => {
  if (!audio.getAttribute("src")) return;
  brokenRecording = audio.dataset.path;
  $("nt-player").hidden = true;
});
$("nt-seek").addEventListener("input", () => {
  audio.currentTime = Number($("nt-seek").value);
});
$("nt-speed").addEventListener("click", () => {
  const next = SPEEDS[(SPEEDS.indexOf(audio.playbackRate) + 1) % SPEEDS.length];
  audio.playbackRate = next;
  $("nt-speed").textContent = `${next}×`;
});

// ---------------------------------------------------------------- actions

async function copyNote() {
  try {
    await invoke("copy_text", { text: body });
    toast("Copied");
  } catch (e) {
    toast(String(e), true);
  }
}

async function sendNote() {
  if ($("nt-send").disabled || $("nt-send").hidden) return;
  await saveNow();
  try {
    // Hides this window, switches to the app and pastes. On failure the
    // window comes back with the note on the clipboard.
    await invoke("notes_send", { text: body });
  } catch (e) {
    toast(String(e), true);
  }
}

async function togglePin() {
  if (!current.path) return;
  const open = notes.find((n) => n.path === current.path);
  await setPinned(current.path, !open?.pinned);
}

async function setPinned(path, pinned) {
  try {
    await invoke("notes_pin", { path, pinned });
    await refresh();
    toast(pinned ? "Pinned to the top of the list" : "Unpinned");
  } catch (e) {
    toast(String(e), true);
  }
}

function startRename() {
  if (!current.path || locked()) return;
  const name = current.path.split("/").pop().replace(/\.md$/i, "");
  $("nt-title").hidden = true;
  $("nt-meta").hidden = true;
  $("nt-rename").hidden = false;
  const input = $("nt-rename-input");
  input.value = name;
  input.focus();
  input.select();
}

function endRename() {
  $("nt-rename").hidden = true;
  $("nt-title").hidden = false;
  $("nt-meta").hidden = false;
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
  if (!current.path || locked()) return;
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

function reveal() {
  invoke("notes_reveal", { path: current.path }).catch((err) => toast(String(err), true));
}

async function toggleMeeting() {
  // Stopping: what was just typed beside it goes into the summary too.
  if (meeting) await flushLiveNotes();
  invoke("meeting_toggle").catch((err) => toast(String(err), true));
}

function pickFile() {
  invoke("notes_transcribe_pick").catch((err) => toast(String(err), true));
}

async function askForLink() {
  const url = await askText("Transcribe a link", {
    detail: "A link to an audio or video file, or a page such as YouTube.",
    placeholder: "https://",
    action: "Transcribe",
  });
  if (url) transcribeLink(url);
}

async function setOnTop(on) {
  try {
    await invoke("notes_keep_on_top", { on });
    onTop = on;
    $("nt-on-top").setAttribute("aria-checked", String(on));
    toast(on ? "Notes stays on top of other apps" : "Notes no longer stays on top");
  } catch (e) {
    toast(String(e), true);
  }
}

// ---------------------------------------------------------------- an app's audio

/** Icons for the apps in the picker, by id, once looked up. */
const appIcons = new Map();

async function pickApp() {
  if (meeting) return toast(meeting.app ? "Already recording." : "A meeting is recording.");
  const dialog = $("nt-apps");
  if (!dialog.open) dialog.showModal();
  await listApps();
}

async function listApps() {
  const list = $("nt-apps-list");
  list.replaceChildren(Object.assign(document.createElement("p"), { className: "nt-hint", textContent: "Looking for apps that play sound…" }));
  let apps = [];
  try {
    apps = (await invoke("audio_apps")) ?? [];
  } catch (e) {
    toast(String(e), true);
  }
  const everything = { id: "", name: "All of this computer's sound", playing: false, sources: [], all: true };
  const rows = [...apps, everything].map((a) => {
    const b = document.createElement("button");
    b.className = "nt-app";
    b.setAttribute("role", "option");
    const iconUrl = appIcons.get(a.id);
    if (iconUrl) {
      b.append(Object.assign(document.createElement("img"), { src: iconUrl, alt: "" }));
    } else {
      const ic = document.createElement("span");
      ic.className = "nt-app-ic";
      ic.dataset.app = a.id;
      ic.append(icon("sound"));
      b.append(ic);
    }
    const name = document.createElement("span");
    name.className = "nt-app-name";
    name.textContent = a.name;
    b.append(name);
    if (!a.all) {
      const state = document.createElement("span");
      state.className = `nt-app-state${a.playing ? " on" : ""}`;
      state.textContent = a.playing ? "Playing" : "Not playing now";
      b.append(state);
    }
    b.addEventListener("click", () => startApp(a));
    return b;
  });
  if (!apps.length) {
    rows.unshift(Object.assign(document.createElement("p"), {
      className: "nt-hint",
      textContent: "No app has played sound yet. Start playing something, then look again.",
    }));
  }
  list.replaceChildren(...rows);
  list.querySelector("button")?.focus();
  // Icons, once each.
  const missing = apps.map((a) => a.id).filter((id) => id && !appIcons.has(id));
  if (missing.length) {
    invoke("app_info", { ids: missing })
      .then((infos) => {
        for (const info of infos ?? []) {
          if (!info.icon) continue;
          appIcons.set(info.id, info.icon);
          for (const slot of list.querySelectorAll(`.nt-app-ic[data-app="${CSS.escape(info.id)}"]`)) {
            slot.replaceWith(Object.assign(document.createElement("img"), { src: info.icon, alt: "" }));
          }
        }
      })
      .catch(() => {});
  }
}

/** Transcribes the microphone alone into a note, hands-free. */
async function recordMicrophone() {
  if (meeting) return toast(meeting.app ? "Already recording." : "A meeting is recording.");
  try {
    await invoke("meeting_start_microphone");
  } catch (e) {
    toast(String(e), true);
  }
}

async function startApp(a) {
  $("nt-apps").close();
  try {
    await invoke("meeting_start_app", { name: a.all ? "This computer's sound" : a.name, sources: a.sources });
  } catch (e) {
    toast(String(e), true);
  }
}

$("nt-apps-refresh").addEventListener("click", listApps);
$("nt-apps-cancel").addEventListener("click", () => $("nt-apps").close());

function editRewrites() {
  invoke("notes_edit_transforms").catch((err) => toast(String(err), true));
}

async function showHistory(show) {
  if (show && (!current.path || locked())) return;
  if (show) closeReview();
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
  $("nt-versions-list").replaceChildren(...versions.map(versionRow));
  $("nt-versions-empty").hidden = versions.length > 0;
  $("nt-history-back").focus();
}

function versionRow(v) {
  const li = document.createElement("li");
  const preview = document.createElement("div");
  preview.className = "nt-preview";
  preview.textContent = v.preview;
  const meta = document.createElement("div");
  meta.className = "nt-vmeta";
  const time = document.createElement("time");
  time.dateTime = new Date(v.at).toISOString();
  const d = new Date(v.at);
  time.textContent = `${when(v.at)}${d.toDateString() === new Date().toDateString() ? "" : `, ${d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`}`;
  const restore = button("Restore", async () => {
    try {
      showNote(await invoke("notes_restore", { path: current.path, at: v.at }));
      toast("Restored. The version it replaced is in Versions.");
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

// ---------------------------------------------------------------- rewrites

/** Saved instructions, from Settings → Words. */
let transforms = [];

/** "Ctrl+Alt+Digit1" → "⌃⌥1" on a Mac, "Ctrl+Alt+1" elsewhere. */
function shortcutLabel(s) {
  const parts = s.split("+");
  const key = parts.pop().replace(/^Key/, "").replace(/^Digit/, "");
  if (!isMac) return [...parts, key].join("+");
  const symbol = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Cmd: "⌘" };
  return parts.map((m) => symbol[m] ?? m).join("") + key;
}

async function loadTransforms() {
  try {
    transforms = await invoke("notes_transforms") ?? [];
  } catch (e) {
    toast(String(e), true);
  }
  // The meeting summary has its own place, in a meeting's Summary tab.
  const chips = transforms.filter((t) => t.id !== "meeting-summary").map((t) => {
    const b = document.createElement("button");
    b.className = "nt-chip";
    b.textContent = t.name;
    b.title = t.shortcut ? `${t.name} (${shortcutLabel(t.shortcut)})` : t.name;
    b.addEventListener("click", () => runTransform(t.id));
    return b;
  });
  $("nt-rewrite-chips").replaceChildren(...chips);
  updateHeader();
}

/** The rewrite being looked at, or null: the text it's of (the selection
 *  or the whole note), the note as it was, and each try. */
let review = null;

function canRewrite() {
  return !$("nt-edit-wrap").hidden && !locked() && !review;
}

/** Starts a rewrite of the selection, or of the whole note: transform
 *  `id`, or (with none) only what `extra` asks for. */
async function runTransform(id, { extra = null } = {}) {
  if (!canRewrite()) {
    if (!review) toast("Open a note to rewrite it.");
    return;
  }
  showHistory(false);
  await saveNow();
  const sel = editor.selection;
  const whole = sel.from === sel.to;
  const text = editor.text;
  const source = whole ? text : text.slice(sel.from, sel.to);
  if (!source.trim()) return toast("There's no text to change.");
  review = {
    path: current.path,
    tab,
    before: text,
    from: whole ? 0 : sel.from,
    to: whole ? text.length : sel.to,
    whole,
    source,
    tries: [],
    shown: -1,
    busy: false,
  };
  showReview(true);
  if (id || extra) await rewrite(id, extra);
  else {
    renderReview();
    $("nt-review-extra").focus();
  }
}

/** One try: transform `id` with `extra` on the original text. */
async function rewrite(id, extra) {
  if (!review || review.busy) return;
  const t = transforms.find((x) => x.id === id);
  const name = t?.name ?? "Rewrite";
  const asked = review;
  asked.busy = { name };
  renderReview();
  let out;
  try {
    out = await invoke("notes_transform", { path: asked.path, id: id ?? "", text: asked.source, extra });
  } catch (e) {
    toast(String(e), true);
  }
  if (review !== asked) return;
  asked.busy = false;
  if (out?.unchanged) toast(`${name}: ${out.unchanged}`);
  if (!out || out.unchanged) {
    if (!asked.tries.length && id) return closeReview();
    return renderReview();
  }
  let label = extra ? (t ? `${name}, your way` : "Your way") : name;
  const same = asked.tries.filter((x) => x.label === label || x.label.startsWith(`${label} `)).length;
  if (same) label = `${label} ${same + 1}`;
  asked.tries.push({ label, text: out.text, place: t?.place ?? "replace", id: id ?? "", extra });
  asked.shown = asked.tries.length - 1;
  $("nt-review-extra").value = "";
  renderReview();
  $("nt-review-keep").focus();
}

function showReview(show) {
  $("nt-review").hidden = !show;
  $("nt-editor").hidden = show;
  editor.readOnly = show || locked();
  if (!show) {
    $("nt-review-extra").value = "";
    updateHeader();
  }
}

function closeReview() {
  if (!review) return;
  review = null;
  showReview(false);
  render({ reload: true });
}

function words(text) {
  return (text.match(/[\p{L}\p{N}]+/gu) ?? []).length;
}

function renderReview() {
  if (!review) return;
  const r = review;
  const shown = r.tries[r.shown];
  const name = r.busy ? `${r.busy.name}…` : shown?.label ?? "Rewrite";
  $("nt-review-name").textContent = name;
  $("nt-review-info").textContent = r.busy
    ? "Working on it"
    : !r.tries.length
      ? "Say how to change it, below."
      : shown
        ? shown.place === "below"
          ? "Goes below the text"
          : `${words(r.source).toLocaleString()} → ${words(shown.text).toLocaleString()} words`
        : "As it was";
  // Original, then each try.
  const seg = $("nt-review-versions");
  seg.hidden = !r.tries.length;
  const buttons = [{ label: "Original", i: -1 }, ...r.tries.map((x, i) => ({ label: x.label, i }))].map(({ label, i }) => {
    const b = document.createElement("button");
    b.textContent = label;
    b.setAttribute("aria-pressed", String(i === r.shown));
    b.disabled = !!r.busy;
    b.addEventListener("click", () => {
      r.shown = i;
      renderReview();
    });
    return b;
  });
  seg.replaceChildren(...buttons);
  const showDiff = $("nt-review-diff").checked;
  $("nt-review-diff-wrap").hidden = !shown || shown.place === "below";
  $("nt-review-keep").disabled = !shown || !!r.busy;
  $("nt-review-keep").textContent = shown?.place === "below" ? "Add it below" : "Keep this";
  $("nt-review-retry").disabled = !!r.busy;
  const out = $("nt-review-text");
  out.classList.toggle("nt-working", !!r.busy && !shown);
  if (!shown) {
    out.textContent = r.source;
  } else if (shown.place === "below" || !showDiff) {
    out.textContent = shown.text;
  } else {
    out.replaceChildren(...diffNodes(r.source, shown.text));
  }
}

/** Keeps the try shown: in place of the text (⌘Z undoes it), or below it. */
function keepRewrite() {
  const r = review;
  const shown = r?.tries[r.shown];
  if (!shown || r.busy) return;
  const changed = current.path !== r.path || tab !== r.tab || editor.text !== r.before;
  closeReview();
  if (changed) {
    invoke("copy_text", { text: shown.text }).catch(() => {});
    return toast("The note changed meanwhile, so the rewrite was copied instead.");
  }
  editor.focus();
  if (shown.place === "below") {
    const before = editor.text.slice(0, r.to);
    const insert = `${before.endsWith("\n") ? "" : "\n"}\n---\n\n${shown.text}`;
    editor.replace(r.to, r.to, insert, { select: false });
  } else {
    editor.replace(r.from, r.to, shown.text, { select: !r.whole });
  }
  toast(`Kept. ${isMac ? "⌘Z" : "Ctrl+Z"} undoes it; the note before is in Versions.`);
}

$("nt-review-keep").addEventListener("click", keepRewrite);
$("nt-review-back").addEventListener("click", () => {
  closeReview();
  focusNote();
});
$("nt-review-diff").addEventListener("change", renderReview);
$("nt-review-again").addEventListener("submit", (e) => {
  e.preventDefault();
  if (!review) return;
  const extra = $("nt-review-extra").value.trim() || null;
  const last = review.tries[review.shown] ?? review.tries[review.tries.length - 1];
  const id = last?.id ?? "";
  if (!id && !extra) return $("nt-review-extra").focus();
  rewrite(id, extra ?? last?.extra ?? null);
});
$("nt-sayhow").addEventListener("click", () => runTransform("", {}));

// A transform's shortcut pressed while Notes is in front.
listen("notes-run-transform", (event) => runTransform(event.payload));

/** Words, spaces and marks, for comparing two texts. */
function tokens(text) {
  return text.match(/\s+|[\p{L}\p{N}_'’]+|[^\s\p{L}\p{N}_]/gu) ?? [];
}

/** The edits that turn token list `a` into `b`, as [op, token] with op
 *  "=", "-" or "+" (Myers' diff). Null when they differ too much to show
 *  usefully. */
function diffTokens(a, b, limit = 3000) {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const A = a.slice(start, endA);
  const B = b.slice(start, endB);
  const n = A.length;
  const m = B.length;
  const max = n + m;
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  const trace = [];
  let found = -1;
  outer: for (let d = 0; d <= max; d++) {
    if (d > limit) return null;
    trace.push(v.slice(off - d - 1, off + d + 2));
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && A[x] === B[y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break outer;
      }
    }
  }
  const ops = [];
  let x = n;
  let y = m;
  for (let d = found; d > 0; d--) {
    const vv = trace[d];
    const get = (k) => vv[k + d + 1];
    const k = x - y;
    const prevK = k === -d || (k !== d && get(k - 1) < get(k + 1)) ? k + 1 : k - 1;
    const prevX = get(prevK);
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      ops.push(["=", A[x - 1]]);
      x--;
      y--;
    }
    if (x === prevX) ops.push(["+", B[prevY]]);
    else ops.push(["-", A[prevX]]);
    x = prevX;
    y = prevY;
  }
  while (x > 0 && y > 0) {
    ops.push(["=", A[x - 1]]);
    x--;
    y--;
  }
  ops.reverse();
  return [...a.slice(0, start).map((t) => ["=", t]), ...ops, ...a.slice(endA).map((t) => ["=", t])];
}

/** `after` with what changed from `before` marked: removed words struck
 *  through, added ones highlighted. */
function diffNodes(before, after) {
  const ops = diffTokens(tokens(before), tokens(after));
  if (!ops) return [document.createTextNode(after)];
  // Runs of one kind; a space between two changes joins them.
  const runs = [];
  for (const [op, t] of ops) {
    const last = runs[runs.length - 1];
    if (last && last.op === op) last.text += t;
    else runs.push({ op, text: t });
  }
  for (let i = 1; i < runs.length - 1; i++) {
    const r = runs[i];
    if (r.op === "=" && /^[ \t]+$/.test(r.text) && runs[i - 1].op !== "=" && runs[i + 1].op !== "=") {
      r.op = "~";
    }
  }
  const nodes = [];
  let removed = "";
  let added = "";
  const flush = () => {
    if (removed) {
      const del = document.createElement("del");
      del.textContent = removed;
      nodes.push(del);
    }
    if (added) {
      const ins = document.createElement("ins");
      ins.textContent = added;
      nodes.push(ins);
    }
    removed = "";
    added = "";
  };
  for (const r of runs) {
    if (r.op === "-") removed += r.text;
    else if (r.op === "+") added += r.text;
    else if (r.op === "~") {
      removed += r.text;
      added += r.text;
    } else {
      flush();
      nodes.push(document.createTextNode(r.text));
    }
  }
  flush();
  return nodes;
}

// ---------------------------------------------------------------- ask

const askState = {
  scope: "all",
  /** Questions and answers: { q, a, done, error, sources, asked, recent }. */
  log: [],
  next: 1,
};

function openAsk({ scope = null, question = null, recent = false } = {}) {
  $("nt-askpanel").hidden = false;
  $("nt-ask").setAttribute("aria-pressed", "true");
  if (scope) setAskScope(scope);
  else if (askState.log.length === 0) setAskScope(current.path ? "note" : "all");
  renderAsk();
  if (question) ask(question, { recent });
  else $("nt-ask-input").focus();
}

function closeAsk() {
  $("nt-askpanel").hidden = true;
  $("nt-ask").setAttribute("aria-pressed", "false");
  focusNote();
}

function setAskScope(scope) {
  if (scope === askState.scope) return;
  askState.scope = scope;
  // A new conversation: the earlier one was about something else.
  askState.log = [];
  renderAsk();
}

async function ask(question, { recent = liveHere() } = {}) {
  question = question.trim();
  if (!question) return;
  let path = null;
  if (askState.scope === "note") {
    await saveNow();
    path = current.path;
    if (!path) setAskScope("all");
  }
  const earlier = askState.log
    .filter((e) => e.done && !e.error && e.a)
    .slice(-ASK_EARLIER)
    .map((e) => [e.q, stripCites(e.a)]);
  const entry = { q: question, a: "", done: false, error: null, sources: [], asked: askState.next++, recent };
  askState.log.push(entry);
  renderAsk();
  try {
    await invoke("notes_ask", { question, path, recent, earlier, asked: entry.asked });
  } catch (e) {
    entry.done = true;
    entry.error = String(e);
    renderAsk();
  }
}

function stripCites(text) {
  return text.replace(/\s?\[\d+\]/g, "");
}

listen("notes-ask", (event) => askUpdated(event.payload));

function askUpdated(u) {
  const entry = askState.log.find((e) => e.asked === u.asked);
  if (!entry) return;
  entry.a = u.answer;
  entry.done = u.done;
  entry.error = u.error;
  entry.sources = u.sources;
  renderAsk();
}

/** Text with **bold** and [n] citations as nodes. */
function inline(text, sources) {
  const out = [];
  const re = /\*\*([^*]+)\*\*|\[(\d+)\]/g;
  let at = 0;
  for (let m; (m = re.exec(text)); ) {
    if (m.index > at) out.push(document.createTextNode(text.slice(at, m.index)));
    if (m[1] != null) {
      const b = document.createElement("b");
      b.textContent = m[1];
      out.push(b);
    } else {
      const source = sources[Number(m[2]) - 1];
      if (source) {
        const cite = document.createElement("button");
        cite.className = "nt-cite";
        cite.textContent = m[2];
        cite.title = source.title;
        cite.setAttribute("aria-label", `Source ${m[2]}: ${source.title}`);
        cite.addEventListener("click", () => openNote(source.path));
        out.push(cite);
      } else {
        out.push(document.createTextNode(m[0]));
      }
    }
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push(document.createTextNode(text.slice(at)));
  return out;
}

/** An answer: paragraphs and lists, with its citations as buttons. */
function answerNodes(text, sources) {
  const nodes = [];
  let list = null;
  for (const line of text.split("\n")) {
    const item = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(line);
    if (item) {
      if (!list) {
        list = document.createElement("ul");
        nodes.push(list);
      }
      const li = document.createElement("li");
      li.append(...inline(item[1], sources));
      list.append(li);
    } else if (line.trim()) {
      list = null;
      const p = document.createElement("p");
      p.append(...inline(line.replace(/^#+\s*/, ""), sources));
      nodes.push(p);
    } else {
      list = null;
    }
  }
  return nodes;
}

function renderAsk() {
  for (const b of document.querySelectorAll("#nt-askpanel [data-scope]")) {
    b.setAttribute("aria-pressed", String(b.dataset.scope === askState.scope));
    if (b.dataset.scope === "note") b.disabled = !current.path;
  }
  $("nt-ask-input").placeholder = askState.log.length
    ? "Ask a follow-up"
    : askState.scope === "note"
      ? `Ask about “${titleOf(body) || "this note"}”`
      : "Ask your notes";
  const rows = [];
  if (!askState.log.length) {
    const hint = document.createElement("p");
    hint.className = "nt-hint";
    hint.textContent = askState.scope === "note"
      ? "Ask about this note: what was decided, who's doing what, or anything else in it."
      : "Ask about anything in your notes. Answers say which notes they came from.";
    rows.push(hint);
  }
  for (const e of askState.log) {
    const q = document.createElement("div");
    q.className = "nt-q";
    q.textContent = e.q;
    rows.push(q);
    const a = document.createElement("div");
    a.className = `nt-a${e.error ? " nt-error" : ""}`;
    if (e.error) a.textContent = e.error;
    else if (!e.a) a.replaceChildren(Object.assign(document.createElement("span"), { className: "nt-pending", textContent: "Thinking…" }));
    else a.replaceChildren(...answerNodes(e.a, e.sources));
    rows.push(a);
    if (e.done && !e.error && e.a) {
      const cited = new Set([...e.a.matchAll(/\[(\d+)\]/g)].map((m) => Number(m[1])));
      const sources = e.sources.map((s, i) => ({ ...s, n: i + 1 })).filter((s) => cited.has(s.n) || e.sources.length === 1);
      if (sources.length) {
        const box = document.createElement("div");
        box.className = "nt-sources";
        for (const s of sources) {
          const b = document.createElement("button");
          b.className = "nt-source";
          const n = document.createElement("span");
          n.className = "nt-cite";
          n.textContent = String(s.n);
          const title = document.createElement("span");
          title.textContent = s.title || s.path;
          b.append(n, title);
          b.title = `Open ${s.title || s.path}`;
          b.addEventListener("click", () => openNote(s.path));
          box.append(b);
        }
        rows.push(box);
      }
      const acts = document.createElement("div");
      acts.className = "nt-a-acts";
      acts.append(
        button("Copy", () => invoke("copy_text", { text: stripCites(e.a) }).then(() => toast("Copied")).catch((err) => toast(String(err), true))),
      );
      const add = button("Add to this note", () => addToNote(stripCites(e.a)));
      add.disabled = $("nt-edit-wrap").hidden || locked() || !!review;
      acts.append(add);
      rows.push(acts);
    }
  }
  const log = $("nt-ask-log");
  log.replaceChildren(...rows);
  log.scrollTop = log.scrollHeight;
}

function addToNote(text) {
  if ($("nt-edit-wrap").hidden || locked() || review) return;
  const end = editor.text.length;
  const before = editor.text;
  const gap = !before.trim() ? "" : before.endsWith("\n\n") ? "" : before.endsWith("\n") ? "\n" : "\n\n";
  editor.focus();
  editor.replace(end, end, `${gap}${text.trim()}\n`, { select: false });
  toast("Added to the end of the note");
}

$("nt-ask").addEventListener("click", () => ($("nt-askpanel").hidden ? openAsk() : closeAsk()));
$("nt-ask-close").addEventListener("click", closeAsk);
$("nt-ask-clear").addEventListener("click", () => {
  askState.log = [];
  renderAsk();
  $("nt-ask-input").focus();
});
for (const b of document.querySelectorAll("#nt-askpanel [data-scope]")) {
  b.addEventListener("click", () => setAskScope(b.dataset.scope));
}
$("nt-ask-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const input = $("nt-ask-input");
  const q = input.value;
  input.value = "";
  ask(q);
});
$("nt-missed").addEventListener("click", () =>
  openAsk({ scope: "note", recent: true, question: "What was said in the last five minutes? Just the main points, briefly." }),
);
$("nt-live-ask").addEventListener("click", () => openAsk({ scope: "note" }));

// ---------------------------------------------------------------- ⌘K

/** Everything the window can do, for the palette. */
function actions() {
  const open = notes.find((n) => n.path === current.path);
  const have = !!current.path;
  const list = [
    { title: "New note", icon: "plus", keys: "⌘N", run: newNote },
    { title: "Today's note", icon: "cal", keys: "⌘T", sub: "Today's daily note, made if it isn't there yet", run: openDaily },
    { title: meeting ? "Stop the meeting" : "Record a meeting", icon: meeting ? "stop" : "rec", sub: "Both sides of a call, transcribed as it goes", run: toggleMeeting },
    { title: "Transcribe a file…", icon: "file", keys: "⌘O", sub: "Audio or video. You can also drop one in this window.", run: pickFile },
    { title: "Transcribe a link…", icon: "link", keys: "⌘L", sub: "YouTube and other pages, or a link to a file", run: askForLink },
    { title: "Transcribe the microphone", icon: "mic", sub: "A lecture, a talk or a conversation in the room, as it's said", run: recordMicrophone, off: !!meeting },
    { title: "Transcribe an app's audio…", icon: "sound", sub: "What a browser, a podcast app or a webinar plays, as it plays", run: pickApp, off: !!meeting },
    { title: "Ask your notes", icon: "ask", keys: "⌘J", sub: "An answer from your notes, with the notes it came from", run: () => openAsk({ scope: "all" }) },
  ];
  if (have) list.push({ title: "Ask about this note", icon: "ask", run: () => openAsk({ scope: "note" }) });
  if (canRewrite()) {
    for (const t of transforms.filter((x) => x.id !== "meeting-summary")) {
      list.push({ title: `Rewrite: ${t.name}`, icon: "pen", sub: titleOf(body) || "This note", run: () => runTransform(t.id) });
    }
    list.push({ title: "Rewrite: say how…", icon: "pen", sub: "In your own words: shorter, warmer, in French…", run: () => runTransform("", {}) });
  }
  if (asMeeting && !locked()) list.push({ title: "Summarize the meeting again", icon: "pen", run: summarize });
  if (have && hasRecording(body) && !locked()) {
    list.push({ title: "Transcribe again with another model…", icon: "pen", run: () => transcribeAgain(current.path) });
  }
  list.push({ title: "New folder…", icon: "folder", run: () => newFolder(view.kind === "folder" ? view.value : "") });
  if (have) {
    list.push(
      { title: "Move this note to…", icon: "folder", run: () => moveNoteTo(current.path), off: locked() },
      { title: "Add a tag to this note…", icon: "tag", run: startTag },
      { title: open?.pinned ? "Unpin this note" : "Pin this note", icon: "pin", run: togglePin },
      { title: "Versions of this note", icon: "clock", run: () => showHistory(true), off: locked() },
      { title: "Rename this note…", icon: "note", run: startRename, off: locked() },
    );
  }
  if (body.trim()) list.push({ title: "Copy this note", icon: "copy", run: copyNote });
  if (sendTarget && body.trim()) list.push({ title: `Paste into ${sendTarget}`, icon: "up", keys: "⌘↩", run: sendNote });
  if (have) list.push({ title: isMac ? "Show in Finder" : "Show in File Explorer", icon: "folder", run: reveal });
  list.push(
    { title: $("nt-side").hidden ? "Show the list" : "Hide the list", icon: "side", keys: "⌘E", run: () => toggleList() },
    { title: $("nt-nav").hidden ? "Show folders and tags" : "Hide folders and tags", icon: "folder", keys: "⌘⇧E", run: () => toggleNav() },
    { title: onTop ? "Stop keeping Notes on top" : "Keep Notes on top of other apps", icon: "pin", run: () => setOnTop(!onTop) },
    { title: "Change notes folder…", icon: "folder", run: changeFolder, off: !!job || !!meeting },
    { title: "Edit rewrites…", icon: "pen", sub: "Settings → Words", run: editRewrites },
  );
  if (have) list.push({ title: "Move this note to the Trash", icon: "x", run: deleteNote, off: locked() });
  return list.filter((a) => !a.off);
}

let palette = { rows: [], at: 0 };

function openPalette() {
  setMenus(null);
  $("nt-palette").hidden = false;
  const q = $("nt-pal-q");
  q.value = "";
  renderPalette();
  q.focus();
}

function closePalette({ focus = true } = {}) {
  if ($("nt-palette").hidden) return;
  $("nt-palette").hidden = true;
  if (focus) focusNote();
}

/** Parts of `text`, with each of `words` (folded) marked. */
function marked(text, words) {
  const folded = fold(text);
  if (!words.length || folded.length !== text.length) return [document.createTextNode(text)];
  const spans = [];
  for (const w of words) {
    if (!w) continue;
    for (let i = folded.indexOf(w); i >= 0; i = folded.indexOf(w, i + w.length)) spans.push([i, i + w.length]);
  }
  spans.sort((a, b) => a[0] - b[0]);
  const out = [];
  let at = 0;
  for (const [s, e] of spans) {
    if (s < at) continue;
    if (s > at) out.push(document.createTextNode(text.slice(at, s)));
    const m = document.createElement("mark");
    m.textContent = text.slice(s, e);
    out.push(m);
    at = e;
  }
  if (at < text.length) out.push(document.createTextNode(text.slice(at)));
  return out;
}

/** About 90 characters of `text` around the first of `words`. */
function around(text, words) {
  const flat = text
    .replace(/^#+\s*/gm, "")
    .replace(/\*\*|`/g, "")
    .replace(/\s+/g, " ");
  const folded = fold(flat);
  const at = Math.min(...words.map((w) => folded.indexOf(w)).filter((i) => i >= 0));
  if (!Number.isFinite(at)) return flat.slice(0, 90);
  const start = Math.max(0, at - 30);
  return `${start > 0 ? "…" : ""}${flat.slice(start, start + 90)}${start + 90 < flat.length ? "…" : ""}`;
}

function renderPalette() {
  const query = $("nt-pal-q").value.trim();
  const words = fold(query).split(/\s+/).filter(Boolean);
  const rows = [];
  const groups = [];
  const has = (text) => words.every((w) => fold(text).includes(w));
  const acts = actions().filter((a) => !words.length || has(`${a.title} ${a.sub ?? ""}`));
  let found;
  if (words.length) {
    found = notes
      .map((n) => ({ n, inTitle: has(n.title) }))
      .filter((x) => x.inTitle || has(`${x.n.title}\n${x.n.text}\n${(x.n.tags ?? []).join(" ")}`))
      .sort((a, b) => b.inTitle - a.inTitle)
      .slice(0, 8)
      .map((x) => x.n);
  } else {
    found = notes.filter((n) => n.path !== current.path).slice(0, 5);
  }
  const note = (n) => ({
    title: n.title || "Untitled",
    sub: words.length ? around(n.text, words) : snippetOf(n.text),
    icon: KIND_ICON[kindOf(n.source, n.text)],
    end: when(n.modified),
    run: () => openNote(n.path),
  });
  if (words.length) {
    // Folders and tags by name: "acme" finds the Acme folder and #acme.
    const places = [];
    const walk = (nodes, kind) => {
      for (const node of nodes) {
        if (has(node.path)) {
          places.push({
            title: kind === "folder" ? node.name : `#${node.path}`,
            sub: kind === "folder" ? `${node.path.split("/").join(" › ")} · ${node.count} ${node.count === 1 ? "note" : "notes"}` : `${node.count} ${node.count === 1 ? "note" : "notes"}`,
            icon: kind === "folder" ? "folder" : "tag",
            run: () => {
              toggleNav(true);
              showView({ kind, value: node.path });
            },
          });
        }
        walk(node.children, kind);
      }
    };
    walk(folderTree(), "folder");
    walk(tagTree(), "tag");
    if (acts.length) groups.push(["Actions", acts.slice(0, 6)]);
    if (places.length) groups.push(["Folders & tags", places.slice(0, 6)]);
    if (found.length) groups.push(["Notes", found.map(note)]);
    groups.push(["Ask", [{ title: `Ask your notes about “${query}”`, icon: "ask", keys: "⌘↩", sub: "An answer from your notes, with the notes it came from", run: () => openAsk({ scope: "all", question: query }) }]]);
  } else {
    if (found.length) groups.push(["Recent", found.map(note)]);
    groups.push(["Actions", acts]);
  }
  const list = $("nt-pal-list");
  const nodes = [];
  for (const [name, items] of groups) {
    const h = document.createElement("div");
    h.className = "nt-pal-group";
    h.textContent = name;
    h.setAttribute("role", "presentation");
    nodes.push(h);
    for (const item of items) {
      const i = rows.length;
      rows.push(item);
      const row = document.createElement("div");
      row.className = "nt-pal-row";
      row.id = `nt-pal-${i}`;
      row.setAttribute("role", "option");
      const ic = document.createElement("span");
      ic.className = "nt-pal-ic";
      ic.append(icon(item.icon));
      const text = document.createElement("span");
      text.className = "nt-pal-text";
      const title = document.createElement("span");
      title.className = "nt-pal-title";
      title.append(...marked(item.title, words));
      text.append(title);
      if (item.sub) {
        const sub = document.createElement("span");
        sub.className = "nt-pal-sub";
        sub.append(...marked(item.sub, words));
        text.append(sub);
      }
      row.append(ic, text);
      if (item.keys) row.append(keysLabel(item.keys));
      else if (item.end) row.append(Object.assign(document.createElement("span"), { className: "nt-pal-end", textContent: item.end }));
      row.addEventListener("mousemove", () => selectRow(i));
      row.addEventListener("click", () => runRow(i));
      nodes.push(row);
    }
  }
  if (!rows.length) {
    nodes.push(Object.assign(document.createElement("p"), { className: "nt-pal-empty", textContent: "Nothing matches." }));
  }
  list.replaceChildren(...nodes);
  palette = { rows, at: 0 };
  selectRow(0);
}

function selectRow(i) {
  if (!palette.rows.length) return;
  palette.at = (i + palette.rows.length) % palette.rows.length;
  for (const row of $("nt-pal-list").querySelectorAll(".nt-pal-row")) {
    const on = row.id === `nt-pal-${palette.at}`;
    row.setAttribute("aria-selected", String(on));
    if (on) row.scrollIntoView({ block: "nearest" });
  }
  $("nt-pal-q").setAttribute("aria-activedescendant", `nt-pal-${palette.at}`);
}

function runRow(i) {
  const item = palette.rows[i];
  if (!item) return;
  closePalette({ focus: false });
  item.run();
}

$("nt-pal-q").addEventListener("input", renderPalette);
$("nt-pal-q").addEventListener("keydown", (e) => {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    selectRow(palette.at + 1);
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    selectRow(palette.at - 1);
  } else if (e.key === "Enter" && mod) {
    e.preventDefault();
    const q = $("nt-pal-q").value.trim();
    closePalette({ focus: false });
    openAsk({ scope: "all", question: q || null });
  } else if (e.key === "Enter") {
    e.preventDefault();
    runRow(palette.at);
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    closePalette();
  }
});
$("nt-palette").addEventListener("mousedown", (e) => {
  if (e.target === $("nt-palette")) closePalette();
});
$("nt-find").addEventListener("click", openPalette);

/** A shortcut as a key cap: ⌘K on a Mac, Ctrl+K elsewhere. */
function keysLabel(keys) {
  const kbd = document.createElement("kbd");
  kbd.className = "nt-kbd";
  kbd.textContent = isMac ? keys : keys.replace("⌘", "Ctrl+").replace("↩", "Enter");
  return kbd;
}

// ---------------------------------------------------------------- transcribing a file

/** The file being transcribed, or null. */
let job = null;

/** "12:30", or "1:02:30" past an hour. */
function clock(secs) {
  const s = Math.floor(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const rest = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${rest}` : `${m}:${rest}`;
}

async function showJob(next) {
  const before = job;
  job = next;
  $("nt-job").hidden = !job;
  if (job?.downloading) {
    // A link, before there's anything to transcribe (or a note to open).
    $("nt-job-bar").removeAttribute("value");
    $("nt-job-text").textContent = `Downloading ${job.file}…`;
    updateHeader();
    return;
  }
  if (job) {
    const progress = $("nt-job-bar");
    let done;
    if (job.total) {
      progress.value = Math.min(job.done / job.total, 1);
      done = `${Math.floor(progress.value * 100)}%`;
    } else {
      progress.removeAttribute("value");
      done = `${clock(job.done)} so far`;
    }
    const waiting = job.waiting ? ` · ${job.waiting} more waiting` : "";
    $("nt-job-text").textContent = `Transcribing ${job.file} ${job.place} · ${done}${waiting}`;
  }
  // A new transcript opens by itself, unless another note is being worked on.
  const started = job && job.note !== before?.note;
  const wasDownload = before?.downloading;
  if (started && (!before || wasDownload || current.path === before.note || (!current.path && !body.trim()))) {
    await saveNow();
    await refresh();
    await openNote(job.note);
  } else if (current.path && (current.path === job?.note || current.path === before?.note)) {
    await reloadTranscript();
  }
  render({ reload: true });
}

/** The transcript grew: show it, keeping the end in view if it was. */
async function reloadTranscript() {
  let n;
  try {
    n = await invoke("notes_open", { path: current.path });
  } catch {
    return;
  }
  if (!n || n.path !== current.path) return;
  const atEnd = editor.atEnd;
  current.modified = n.modified;
  setBody(n.body, { reload: true });
  if (atEnd && !liveHere()) editor.scrollToEnd();
  if (notes.some((x) => x.path === current.path)) updateListEntry(n.body);
  else await refresh();
}

listen("notes-transcribing", (event) => showJob(event.payload ?? null));
listen("notes-transcribe-message", (event) => toast(event.payload.text, event.payload.error));
listen("notes-dragging", (event) => { $("nt-drop").hidden = !event.payload; });
$("nt-job-stop").addEventListener("click", () => invoke("notes_transcribe_stop").catch((e) => toast(String(e), true)));
$("nt-job-show").addEventListener("click", () => job && openNote(job.note));

/** Transcribes a link; a page (YouTube) first needs the link helper,
 *  downloaded once after asking. */
async function transcribeLink(url) {
  try {
    await invoke("notes_transcribe_url", { url });
  } catch (err) {
    if (String(err) !== "needs-link-helper") return toast(String(err), true);
    const ok = await askConfirm("Download the link helper?", {
      action: "Download",
      danger: false,
      detail: `Transcribing pages such as YouTube needs yt-dlp and a small JavaScript engine (QuickJS), about ${isMac ? 39 : 26} MB from their official GitHub releases. Splashdown keeps them up to date. Links straight to an audio or video file don't need them.`,
    });
    if (!ok) return;
    try {
      await invoke("notes_install_link_helper");
      await invoke("notes_transcribe_url", { url });
    } catch (e) {
      toast(String(e), true);
    }
  }
}

// ---------------------------------------------------------------- meetings: recording

/** The meeting recording now, or null. */
let meeting = null;
/** When the growing meeting note was last re-read (status comes twice a second). */
let meetingReadAt = 0;
/** What's typed beside the meeting, waiting to be written. */
let liveNotesTimer = null;
let liveNotesSaving = Promise.resolve();

function liveNotesEdited() {
  clearTimeout(liveNotesTimer);
  liveNotesTimer = setTimeout(flushLiveNotes, LIVE_NOTES_DELAY);
}

function flushLiveNotes() {
  clearTimeout(liveNotesTimer);
  if (liveNotesTimer == null) return liveNotesSaving;
  liveNotesTimer = null;
  const note = meeting?.note ?? current.path;
  const text = liveEditor.text;
  liveNotesSaving = liveNotesSaving.then(() =>
    invoke("meeting_set_notes", { note, text }).catch((e) => toast(String(e), true)),
  );
  return liveNotesSaving;
}

async function showMeeting(next) {
  const before = meeting;
  meeting = next;
  if (meeting) {
    const text = meeting.question
      ? meeting.question.title
      : meeting.finishing
      ? "Finishing the meeting note…"
      : meeting.warning ?? (meeting.app
        ? `${meeting.app} · ${clock(meeting.elapsed)} · transcribed ${meeting.place}`
        : `Meeting · ${clock(meeting.elapsed)} · transcribed ${meeting.place}${meeting.them_problem ? " · your side only" : ""}`);
    $("nt-meeting-text").textContent = text;
    $("nt-meeting-text").title = meeting.them_problem ?? "";
    $("nt-meeting-keep").hidden = !meeting.warning;
    $("nt-meeting-stop").hidden = meeting.finishing;
    showLevel($("nt-level-you"), meeting.you);
    showLevel($("nt-level-them"), meeting.them);
    showLevel($("nt-live-you"), meeting.you);
    showLevel($("nt-live-them"), meeting.them);
    showHearing(meeting);
    $("nt-live-time").textContent = meeting.finishing ? "Finishing…" : clock(meeting.elapsed);
    $("nt-live-place").textContent = meeting.app
      ? `${meeting.app} · transcribed ${meeting.place}`
      : meeting.them_problem
        ? `Your side only: ${meeting.them_problem}`
        : `Transcribed ${meeting.place}`;
    $("nt-live-them-wrap").hidden = !!meeting.them_problem || !!meeting.app;
    const side = meeting.app === "Microphone" ? "Mic" : meeting.app ? "Sound" : "You";
    $("nt-live-you-label").textContent = side;
    $("nt-levels-them").hidden = !!meeting.app;
    $("nt-levels-you-label").textContent = side;
    $("nt-live-stop").disabled = meeting.finishing;
    $("nt-live-stop-label").textContent = meeting.finishing ? "Finishing…" : "Stop";
    $("nt-live-warning").hidden = !meeting.warning;
    showQuestion(meeting.question);
    $("nt-live-warning-text").textContent = meeting.warning ?? "";
  }
  // A new meeting's note opens by itself.
  if (meeting && meeting.note !== before?.note) {
    await saveNow();
    await refresh();
    if (current.path === meeting.note) render();
    else await openNote(meeting.note);
    liveEditor.focus();
  } else if (!meeting && before && current.path === before.note) {
    // It's done: what was typed beside it is in, and so is the summary.
    await flushLiveNotes();
    tab = "summary";
    await reloadTranscript();
    tab = parseMeeting(body).parts.some((p) => p.key === "summary") ? "summary" : "transcript";
    render();
  } else if (current.path && current.path === meeting?.note) {
    // It grows a turn at a time; re-read it every few seconds.
    if (Date.now() - meetingReadAt > 2000) {
      meetingReadAt = Date.now();
      await reloadTranscript();
    }
  }
  updateHeader();
}

/** Below this (about −55 dBFS) there's no sound to speak of. */
const SILENT = 0.0018;
/** When the recording last had sound, by the clock, for "No sound". */
let soundAt = 0;

/** A level (RMS, 0–1) on a meter: −60 dB to 0 dB across, rising at once
 *  and falling back gently, as level meters do. */
function showLevel(bar, rms) {
  const db = 20 * Math.log10(Math.max(rms, 1e-6));
  const target = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  const shown = Number(bar.dataset.shown ?? 0);
  const next = target >= shown ? target : Math.max(target, shown - 15);
  bar.dataset.shown = String(next);
  bar.style.width = `${next}%`;
  bar.parentElement.setAttribute("aria-valuenow", String(Math.round(next)));
}

/** Whether the recording is hearing anything, in words: no sound at all,
 *  sound with nothing said, or when words were last heard. */
function showHearing(m) {
  const el = $("nt-hearing");
  const loud = Math.max(m.you, m.them) > SILENT;
  if (loud || !soundAt) soundAt = loud ? Date.now() : soundAt || Date.now();
  let text = "";
  let kind = "";
  if (m.finishing) {
    text = "";
  } else if (m.heard != null && m.elapsed - m.heard < 4) {
    text = "Hearing words";
    kind = "words";
  } else if (Date.now() - soundAt > 3000) {
    const who = m.app === "Microphone" ? "the microphone" : m.app ?? "either side";
    text = `No sound from ${who}`;
    kind = "quiet";
  } else if (m.heard != null) {
    const ago = Math.round(m.elapsed - m.heard);
    text = ago < 60 ? `Words ${ago} s ago` : `Words ${Math.floor(ago / 60)} min ago`;
  } else {
    text = "Sound, no words yet";
  }
  el.textContent = text;
  el.className = `nt-hearing${kind ? ` ${kind}` : ""}`;
}

/** A question the meeting's note waits on, with its answers. */
function showQuestion(q) {
  $("nt-live-question").hidden = !q;
  $("nt-live-time").textContent = q ? "Waiting for you" : $("nt-live-time").textContent;
  if (!q) return;
  if ($("nt-live-question-title").textContent === q.title) return;
  $("nt-live-question-title").textContent = q.title;
  $("nt-live-question-text").textContent = q.text;
  $("nt-live-question-acts").replaceChildren(
    ...q.buttons.map((label, index) =>
      button(label, () => invoke("prompt_answer", { index }).catch((e) => toast(String(e), true)), index === 0 ? "btn primary" : "btn"),
    ),
  );
}

listen("meeting-status", (event) => showMeeting(event.payload ?? null));
listen("meeting-message", (event) => toast(event.payload.text, event.payload.error));
$("nt-meeting-stop").addEventListener("click", toggleMeeting);
$("nt-live-stop").addEventListener("click", toggleMeeting);
$("nt-meeting-keep").addEventListener("click", () => invoke("meeting_keep_going").catch(() => {}));
$("nt-live-keep").addEventListener("click", () => invoke("meeting_keep_going").catch(() => {}));
$("nt-meeting-show").addEventListener("click", () => meeting && openNote(meeting.note));

// ---------------------------------------------------------------- menus

/** Opens menu `name` ("more", "transcribe") or, with null, closes them. */
function setMenus(name) {
  hideItemMenu();
  for (const [key, btn, menu] of [["more", "nt-more", "nt-menu"], ["transcribe", "nt-transcribe", "nt-transcribe-menu"]]) {
    const open = key === name;
    $(menu).hidden = !open;
    $(btn).setAttribute("aria-expanded", String(open));
    if (open) $(menu).querySelector("button:not(:disabled)")?.focus();
  }
}

$("nt-more").addEventListener("click", (e) => {
  e.stopPropagation();
  setMenus($("nt-menu").hidden ? "more" : null);
});
$("nt-transcribe").addEventListener("click", (e) => {
  e.stopPropagation();
  setMenus($("nt-transcribe-menu").hidden ? "transcribe" : null);
});
document.addEventListener("click", () => {
  setMenus(null);
  hideItemMenu();
});

const ACTS = {
  rename: startRename,
  reveal,
  folder: changeFolder,
  delete: deleteNote,
  "edit-rewrites": editRewrites,
  "on-top": () => setOnTop(!onTop),
  transcribe: pickFile,
  "transcribe-link": askForLink,
  "transcribe-app": pickApp,
  "transcribe-mic": recordMicrophone,
  meeting: toggleMeeting,
  ask: () => openAsk({ scope: "all" }),
};
for (const menu of [$("nt-menu"), $("nt-transcribe-menu"), $("nt-start")]) {
  menu.addEventListener("click", (e) => {
    const act = e.target.closest("button")?.dataset.act;
    if (!act) return;
    setMenus(null);
    ACTS[act]();
  });
}
// Arrow keys move through an open menu.
for (const menu of [$("nt-menu"), $("nt-transcribe-menu"), $("nt-item-menu")]) {
  menu.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...menu.querySelectorAll("button:not(:disabled)")];
    const at = items.indexOf(document.activeElement);
    items[(at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
  });
}

// ---------------------------------------------------------------- wiring

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
$("nt-folder").addEventListener("click", () => invoke("notes_reveal", { path: null }).catch((e) => toast(String(e), true)));
$("nt-reveal").textContent = isMac ? "Show in Finder" : "Show in File Explorer";
for (const kbd of document.querySelectorAll("kbd[data-keys]")) {
  kbd.textContent = keysLabel(kbd.dataset.keys).textContent;
}

// You used another app while Notes stayed open: Paste into goes there now.
listen("notes-send-target", (event) => {
  sendTarget = event.payload ?? null;
  updateHeader();
});

/** What a voice action asked for once the window is up: "new" or "daily". */
function then(what) {
  if (what === "new") newNote();
  else if (what === "daily") openDaily();
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

// ---------------------------------------------------------------- preview

/** Sample backend for opening this page as a plain file:// page (see
 *  invoke in common.js). */
async function mockInvoke(cmd, args) {
  const hour = 3600 * 1000;
  const meetingBody = "# Weekly product sync · Oct 28, 9:00 AM\n\nRecorded on this computer · transcribed on this computer.\n\n## My notes\n\n- Comparison table → 3 rows\n- Annual: 2 months free?\n\n## Summary\n\n### Overview\nThe team agreed to shorten the pricing page's comparison table and to look at an annual plan.\n\n### Decisions\n- Cut the comparison table to three rows; the rest moves to a details page.\n\n### Action items\n- [ ] Model per-seat pricing with and without the annual discount, by Thursday (Them)\n- [x] Reply to Dana with the plan (You)\n\n## Transcript\n\n**[16:58] Them:** So for the pricing page I think the comparison table is doing more harm than good.\n\n**[17:20] You:** Agreed. Dana said the same in the beta notes.\n\n**[17:41] Them:** And the annual plan. If we do two months free, does that change the team price per seat?\n\n**[18:15] You:** It shouldn't. Can you model both and bring it Thursday?\n\n*Ended at 34:10.*\n\n[Recording](Attachments/2026-10-28%20Weekly%20product%20sync.wav)\n";
  const list = [
    { path: "Work/Launch/2026-10-31 Launch post.md", title: "Launch post", text: "Launch post\n- lead with the hold-to-talk demo #marketing\n- mention it's **free** and open source\n- [ ] ask Dana about the table", modified: Date.now() - 0.2 * hour, pinned: true, folder: "Work/Launch", source: "", tags: ["launch", "marketing"] },
    { path: "Meetings/2026-10-28 Weekly product sync.md", title: "Weekly product sync · Oct 28, 9:00 AM", text: meetingBody, modified: Date.now() - 1 * hour, pinned: false, folder: "Meetings", source: "meeting" },
    { path: "2026-10-31 Call the dentist.md", title: "Call the dentist", text: "Call the dentist\nabout moving Thursday's appointment.", modified: Date.now() - 2 * hour, pinned: false, folder: "", source: "capture" },
    { path: "2026-10-30 Writing for the ear.md", title: "Writing for the ear", text: "# Writing for the ear\n\nTranscribed from <https://www.youtube.com/watch?v=x> (12:04) on this computer.\n\n**[0:00]** Welcome back.", modified: Date.now() - 30 * hour, pinned: false, folder: "", source: "file" },
    { path: "Work/Standup.md", title: "Standup", text: "Standup\nWindows build compiles; testing Monday. #work/acme", modified: Date.now() - 80 * hour, pinned: false, folder: "Work", source: "", tags: ["work/acme"] },
    { path: "Personal/Recipes/Bread.md", title: "Bread", text: "Bread\nflour, water, salt", modified: Date.now() - 300 * hour, pinned: false, folder: "Personal/Recipes", source: "", tags: [] },
  ];
  const open = (path) => {
    const n = list.find((x) => x.path === path) ?? list[0];
    return { path: n.path, body: n.text, modified: n.modified, source: n.source, front_tags: n.path.includes("Launch") ? ["launch"] : [] };
  };
  switch (cmd) {
    case "notes_load":
      return {
        folder: { path: "/Users/you/Documents/Splashdown Notes", label: "Splashdown Notes", synced: null, problem: null },
        notes: list,
        open: open(list[0].path),
        target: "Mail",
      };
    case "notes_list":
      return list;
    case "notes_daily":
      return { path: "Daily/2026-11-01.md", body: "# 2026-11-01\n\n", modified: Date.now(), source: "", front_tags: [] };
    case "notes_folders":
      return ["Meetings", "Personal", "Personal/Recipes", "Work", "Work/Launch", "Archive"];
    case "notes_open":
      return open(args.path);
    case "notes_transcribe_again_models":
      return [{ id: null, label: "The speech model in Settings" }, { id: "parakeet-v3", label: "Parakeet v3 · 25 European languages" }, { id: "large-v3-turbo", label: "Whisper large-v3-turbo · about 100 languages" }];
    case "notes_save":
      return { note: { path: args.path ?? "2026-11-01 New.md", body: args.text, modified: Date.now(), source: "" }, conflict: false };
    case "notes_versions":
      return [{ at: Date.now() - hour, preview: "Launch post\n- lead with the demo" }];
    case "notes_transforms":
      return [
        { id: "clean-up", name: "Clean up", place: "replace", shortcut: null },
        { id: "summary", name: "Summary", place: "below", shortcut: null },
        { id: "todos", name: "To-dos", place: "below", shortcut: "Ctrl+Alt+Digit1" },
        { id: "meeting-summary", name: "Meeting summary", place: "below", shortcut: null },
        { id: "email", name: "Email", place: "replace", shortcut: null },
      ];
    case "notes_transform":
      await new Promise((r) => setTimeout(r, 300));
      return { text: args.text.replace("lead with", "Lead with").replace("mention it's", "Say it's").replace(/- \[ \] ask Dana about the table/, "- [ ] Ask Dana whether the table helps"), unchanged: null };
    case "audio_apps":
      return [
        { id: "com.google.Chrome", name: "Google Chrome", playing: true, sources: [221, 222] },
        { id: "com.spotify.client", name: "Spotify", playing: true, sources: [230] },
        { id: "us.zoom.xos", name: "Zoom", playing: false, sources: [259] },
      ];
    case "app_info":
      return [];
    case "notes_ask":
      setTimeout(() => askUpdated({ asked: args.asked, answer: "Lead with the hold-to-talk demo [1], and say it's free [1]. The meeting agreed to cut the table [2].", done: true, error: null, sources: [{ title: "Launch post", path: list[0].path }, { title: "Weekly product sync", path: list[1].path }] }), 200);
      return null;
    default:
      return null;
  }
}
