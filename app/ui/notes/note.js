"use strict";

// The open note: showing it, the editor, new notes, the header and
// notices.

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
  return (!!job && !job.downloading && job.note === current.path) || liveHere() || (!!carryingOn && carryingOn === current.path);
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
  $("nt-hide-agents").disabled = !current.path || isLocked;
  $("nt-hide-agents").setAttribute("aria-checked", String(!!open?.hidden_from_agents));
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
