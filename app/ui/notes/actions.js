"use strict";

// The note's commands: copy, paste into the app in front, pin, rename,
// delete, change folder, reveal, keep on top, and starting a meeting, a
// file or a link.

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

async function toggleHiddenFromAgents() {
  if (!current.path) return;
  const open = notes.find((n) => n.path === current.path);
  const hidden = !open?.hidden_from_agents;
  await saveNow();
  try {
    await invoke("notes_set_hidden_from_agents", { path: current.path, hidden });
    await refresh();
    toast(hidden ? "Hidden from coding agents" : "Coding agents can reach this note again");
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
