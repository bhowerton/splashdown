"use strict";

// The sidebar: the views (all notes, today's note), the folder and tag
// trees, moving notes and folders, and a note's own tags.

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
    menuItem(`Move to ${TRASH}`, () => deleteFolder(path), { disabled: busy, danger: true }),
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
  const ok = await askConfirm(`Move “${name}” to the ${TRASH}?`, {
    action: `Move to ${TRASH}`,
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
