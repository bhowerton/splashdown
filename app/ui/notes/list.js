"use strict";

// The list of notes on the left: sorting, filtering, pins and selection.

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
