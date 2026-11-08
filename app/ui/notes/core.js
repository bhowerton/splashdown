"use strict";

// The Notes window (notes.html): Markdown files in the notes folder, listed
// on the left and edited on the right. A new note becomes a file once it
// has text; after that it's saved as you type. A meeting's note shows as
// tabs (its summary, transcript and the user's own notes); while it
// records, the transcript grows beside a place to write. ⌘K finds any
// note or action, ⌘J asks the notes a question, and a rewrite shows what
// it changed before it's kept.
//
// Its scripts, in notes/ in load order: this file (shared state, constants
// and saving), then list, folders, note, meeting, actions, app-audio,
// rewrites, ask, palette, transcribe, recording, menus, wiring and preview.
// They are classic scripts sharing one global scope (with common.js, loaded
// first), so a later file may use an earlier one's names at load, not the
// reverse.

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
/** Where a deleted note goes, as this computer calls it. */
const TRASH = isMac ? "Trash" : "Recycle Bin";
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
  // Defined in later files (actions.js, note.js): looked up when called.
  onSend: (...args) => sendNote(...args),
  onPaste: (...args) => pasted(...args),
  onTag: (tag) => {
    toggleNav(true);
    showView({ kind: "tag", value: tag });
  },
});
const liveEditor = NoteEditor.create($("nt-live-notes"), {
  label: "My notes",
  placeholder: "What matters to you in this meeting",
  onChange: (...args) => liveNotesEdited(...args),
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
