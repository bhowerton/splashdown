"use strict";

// Recording a meeting from the Notes window, and its live transcript.

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
    call("meeting_set_notes", { note, text }),
  );
  return liveNotesSaving;
}

/** The note a finished meeting is about to be carried on in. It's locked
 *  from when that's asked for, so nothing typed since is saved over the
 *  line the recording adds to it. */
let carryingOn = null;

/** A finished meeting's note (or a microphone recording's) can take more:
 *  an app's audio or a transcribed file can't. */
function canCarryOn(source) {
  return source === "meeting" || source === "recording";
}

/** Records more into the finished meeting in note `path`, from where it
 *  stopped. */
async function carryOn(path) {
  if (meeting) return toast(meeting.app ? "Already recording." : "A meeting is recording.");
  if (current.path === path) await saveNow();
  carryingOn = path;
  if (current.path === path) render();
  try {
    await invoke("meeting_carry_on", { path });
  } catch (e) {
    carryingOn = null;
    if (current.path === path) render();
    toast(String(e), true);
  }
}

async function showMeeting(next) {
  const before = meeting;
  meeting = next;
  // It started, or couldn't: either way the note is no longer waiting.
  if (carryingOn) {
    const waited = carryingOn;
    carryingOn = null;
    if (!meeting && current.path === waited) render();
  }
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
      button(label, () => call("prompt_answer", { index }), index === 0 ? "btn primary" : "btn"),
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
