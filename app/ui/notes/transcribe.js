"use strict";

// Transcribing a file or a link into a note: the queue and its progress.

/** The file being transcribed, or null. */
let job = null;

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
$("nt-job-stop").addEventListener("click", () => call("notes_transcribe_stop"));
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
