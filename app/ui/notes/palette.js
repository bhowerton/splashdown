"use strict";

// ⌘K: find any note or action.

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
  if (have && hasTimedTranscript(body) && !locked()) {
    list.push(
      { title: "Save subtitles (SRT)…", icon: "down", sub: "For a video player or editor", run: () => exportSubtitles(current.path, "srt") },
      { title: "Save subtitles (VTT)…", icon: "down", sub: "For the web", run: () => exportSubtitles(current.path, "vtt") },
    );
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
