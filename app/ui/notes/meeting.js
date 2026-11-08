"use strict";

// A meeting note: its tabs (summary, transcript, my notes), speakers, summarizing, and playing the recording.

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
  // Their voice from this meeting is kept under the name, when Settings
  // remembers people (nothing happens otherwise).
  invoke("notes_speaker_named", { path: current.path, who, name }).catch(() => {});
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

/** A transcript with times (`**[1:02]** …`), which subtitles are made from
 *  (matches `meeting::Turn::parse`). */
function hasTimedTranscript(text) {
  return /^\*\*\[\d+:\d\d(:\d\d)?\]/m.test(text ?? "");
}

/** Asks where, and saves the note's transcript as subtitles. */
async function exportSubtitles(path, format) {
  try {
    const saved = await invoke("notes_export_subtitles", { path, format });
    if (!saved) return;
    const name = saved.file.split(/[\\/]/).pop();
    toast(saved.approximate ? `Saved ${name}. Times are estimated: transcribe on this computer for exact ones.` : `Saved ${name}`);
  } catch (err) {
    toast(String(err), true);
  }
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
    call("notes_transcribe_again", { path, model });
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
