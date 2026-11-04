"use strict";

// General tab: hotkey pickers and their legend, launch at login, updates,
// microphone, live preview, sounds. Also the Words & language tab's name,
// language and vocabulary (its snippets live in snippets.js).

/** The ask letter when none is set: the first of A, K, J, Q that hands-free
 *  and notes don't use (as `Hotkeys::ask_key`). */
function askDefault(hotkeys) {
  const hands = (hotkeys.hands_free || "h").toLowerCase();
  const note = (hotkeys.note || (hands === "n" ? "m" : "n")).toLowerCase();
  return ["a", "k", "j", "q"].find((c) => c !== hands && c !== note) ?? "a";
}

/** Fallback for a backend that doesn't send `defaults.sound_volume` yet.
 *  Matches splashdown_config's DEFAULT_SOUND_VOLUME. */
const DEFAULT_SOUND_VOLUME = 0.12;
// What "Find notes by meaning" says when it's not busy, and whether its
// progress is being shown.
const MEANING_NOTE = $("nt-meaning-note").textContent;
let meaningShown = false;

function renderGeneral() {
  const s = state.settings;
  renderMicrophones();
  if (!capture) {
    renderKeyPicker("hk-dictation", s.hotkeys.dictation ?? defaultKey("hk-dictation"));
    renderKeyPicker("hk-instruction", s.hotkeys.instruction ?? defaultKey("hk-instruction"));
  }
  renderHotkeyTip();
  setValue($("hk-hands-free"), (s.hotkeys.hands_free || "h").toUpperCase());
  const noteDefault = (s.hotkeys.hands_free || "h").toLowerCase() === "n" ? "m" : "n";
  setValue($("hk-note"), (s.hotkeys.note || noteDefault).toUpperCase());
  setValue($("hk-ask"), (s.hotkeys.ask || askDefault(s.hotkeys)).toUpperCase());
  setValue($("hk-compose"), (s.hotkeys.compose || "").toUpperCase());
  $("gen-compose-panel").checked = s.general.compose_panel === true;
  $("gen-compose-style").checked = s.general.compose_style !== false;
  const notes = s.notes ?? {};
  setValue($("nt-capture-to"), notes.capture_to ?? "inbox");
  $("nt-daily-row").hidden = (notes.capture_to ?? "inbox") !== "daily";
  setValue($("nt-daily-folder"), notes.daily_folder ?? "");
  setValue($("nt-daily-format"), notes.daily_format ?? "");
  $("nt-keep-audio").checked = !!notes.keep_audio;
  setValue($("mt-summary"), notes.meeting_summary ?? "");
  $("mt-keep-audio").checked = !!notes.keep_meeting_audio;
  $("mt-stop-quiet").checked = notes.stop_when_quiet ?? true;
  $("mt-offer").checked = notes.offer_on_calls ?? true;
  $("mt-calendar").checked = !!notes.meeting_calendar;
  $("mt-speakers").checked = !!notes.meeting_speakers;
  $("nt-meaning").checked = !!notes.ask_by_meaning;
  if (notes.ask_by_meaning) showMeaningProgress();
  // EventKit is the Mac's; there's no calendar to read on Windows yet.
  $("mt-calendar-row").hidden = state.platform !== "macos";
  $("mt-pill").checked = notes.meeting_pill ?? state.platform === "windows";
  $("hk-double-tap").checked = s.hotkeys.double_tap === true;
  setValue($("gen-name"), s.general.user_name);
  setValue($("gen-language"), s.general.language ?? "");
  // English-only local models (Nemotron English, Parakeet v2, Whisper *.en)
  // can't hear other languages.
  const speech = "Ok" in state.resolved_speech ? state.resolved_speech.Ok : "";
  const localId = /^On this computer · (.+)$/.exec(speech)?.[1];
  const englishOnly = !!state.speech_models.find((m) => m.id === localId)?.english_only;
  const other = s.general.language && s.general.language !== "en";
  $("gen-language-note").hidden = !(englishOnly && other);
  $("gen-language-note").textContent = "Your speech model only understands English. Choose Nemotron 3.5 or Whisper large-v3-turbo under AI Models → Speech to text → On this computer.";
  // One per line, so a term with a comma in it ("Smith, Jones & Co")
  // comes back whole.
  setValue($("gen-vocab"), s.general.vocabulary.join("\n"));
  showLearned(s.general.learned_words ?? []);
  $("hist-keep").checked = s.general.keep_history === true; // on the History tab; off unless turned on
  $("hist-audio").checked = s.general.keep_audio === true;
  $("gen-near-cursor").checked = s.general.overlay_near_cursor !== false;
  $("gen-whisper").checked = s.general.whisper_mode === true;
  renderMixedLanguages(s.general.languages ?? []);
  $("gen-mini-bar").checked = s.general.mini_bar === true;
  $("gen-noise").checked = s.general.noise_reduction === true;
  setValue($("gen-other-audio"), s.general.other_sounds ?? "lower");
  // A number of days set by hand in config.toml gets its own choice.
  const days = String(s.general.history_days ?? "");
  if (![...$("hist-days").options].some((o) => o.value === days)) $("hist-days").append(new Option(`${days} days`, days));
  setValue($("hist-days"), days);
  $("ins-keep").checked = s.general.insights !== false;
  $("hist-audio").disabled = !$("hist-keep").checked;
  $("gen-learn").checked = s.general.learn_vocabulary !== false;
  $("gen-updates").checked = s.general.check_for_updates !== false;
  $("gen-sounds").checked = s.general.sounds !== false;
  const volume = s.general.sound_volume ?? state.defaults.sound_volume ?? DEFAULT_SOUND_VOLUME;
  setValue($("gen-volume"), Math.round(volume * 100));
  $("gen-volume").disabled = !$("gen-sounds").checked;
  $("gen-live").checked = s.general.live_preview !== false;
  $("gen-stream").checked = s.general.stream_typing !== false;
  const note = $("gen-live-note");
  // Only the model the user chose is ever loaded, so the preview needs
  // speech-to-text on this computer.
  const resolved = "Ok" in state.resolved_speech ? state.resolved_speech.Ok : "";
  const ready = resolved.startsWith("On this computer");
  note.textContent = !ready && $("gen-live").checked
    ? "Works when speech-to-text runs on this computer: choose it under AI Models → Speech to text"
    : "Shown in the status pill as you speak";
  note.classList.toggle("warn-text", !ready && $("gen-live").checked);
}

// ---------------------------------------------------------------- languages

/** Languages that can be mixed (the same list as "Language you speak"). */
const MIX_LANGUAGES = [
  ["en", "English"], ["es", "Spanish"], ["fr", "French"], ["de", "German"], ["it", "Italian"],
  ["pt", "Portuguese"], ["nl", "Dutch"], ["ja", "Japanese"], ["zh", "Chinese"], ["ko", "Korean"], ["hi", "Hindi"],
];

function renderMixedLanguages(chosen) {
  // Only with "Detect automatically": a fixed language means one language.
  $("gen-languages-row").hidden = !!$("gen-language").value;
  $("gen-languages").replaceChildren(
    ...MIX_LANGUAGES.map(([code, name]) => {
      const label = document.createElement("label");
      label.className = "lang-chip";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.value = code;
      box.checked = chosen.includes(code);
      box.onchange = () => save();
      label.append(box, document.createTextNode(name));
      return label;
    })
  );
}

/** The ticked languages, for collect(). */
function mixedLanguages() {
  return [...document.querySelectorAll("#gen-languages input:checked")].map((b) => b.value);
}

$("gen-language").addEventListener("change", () => ($("gen-languages-row").hidden = !!$("gen-language").value));

// ---------------------------------------------------------------- hotkeys

// Each hotkey is a key picker: a button showing the key as a keycap (click
// it, then press the key you want) and a ▾ list for keys a web page can't
// see (Fn) or that are easier to pick than press. The value lives in the
// picker's data-value; collect() reads it from there.

/** Browser key codes for the modifier keys a hotkey can use. Fn can't be
 *  seen by web pages, so it's only offered in the list. */
const CODE_TO_KEY = {
  AltRight: "right-alt", AltLeft: "left-alt",
  MetaRight: "right-meta", MetaLeft: "left-meta",
  ControlRight: "right-control", ControlLeft: "left-control",
  ShiftRight: "right-shift", ShiftLeft: "left-shift",
};
/** Mouse buttons by `MouseEvent.button`: 3 is Back, 4 is Forward. */
const MOUSE_TO_KEY = { 3: "mouse-4", 4: "mouse-5" };
/** Mac modifier symbols, by the part after "left-"/"right-". */
const KEY_SYMBOL = { alt: "⌥", meta: "⌘", control: "⌃", shift: "⇧" };

function defaultKey(id) {
  return id === "hk-dictation" ? defaultDictationKey(state.platform) : "right-shift";
}

function keyLabel(value) {
  return state.key_options.find((k) => k.value === value)?.label ?? value;
}

/** "Right ⌥ Option" on a Mac; the plain label elsewhere. */
function keyCap(value) {
  const label = keyLabel(value);
  const sym = KEY_SYMBOL[value.replace(/^(left|right)-/, "")];
  if (state.platform !== "macos" || !sym) return label;
  const [side, ...rest] = label.split(" ");
  return `${side} ${sym} ${rest.join(" ")}`;
}

/** The key's short name for the gesture legend: "⌥", "Ctrl", "Mouse 4". */
function keyShort(value) {
  if (value.startsWith("mouse-")) return `Mouse ${value.slice(6)}`;
  const sym = KEY_SYMBOL[value.replace(/^(left|right)-/, "")];
  if (state.platform === "macos" && sym) return sym;
  return keyLabel(value).replace(/^(Left|Right) /, "");
}

function renderKeyPicker(id, value) {
  const picker = $(id);
  picker.dataset.value = value;
  picker.classList.remove("recording");
  const name = picker.dataset.name;

  const main = document.createElement("button");
  main.type = "button";
  main.className = "kp-main";
  main.setAttribute("aria-label", `${name[0].toUpperCase()}${name.slice(1)}: ${keyLabel(value)}. Click, then press the key you want.`);
  const cap = document.createElement("span");
  cap.className = "kbd";
  cap.textContent = keyCap(value);
  const hint = document.createElement("span");
  hint.className = "kp-hint";
  hint.textContent = "Click to change";
  main.append(cap, hint);
  main.onclick = () => (capture?.picker === picker ? stopCapture() : startCapture(picker));

  // The ▾ list: a native menu (with a tick on the current key) behind the
  // chevron, so it's keyboard- and VoiceOver-friendly.
  const more = document.createElement("span");
  more.className = "kp-more";
  const chevron = document.createElement("span");
  chevron.className = "kp-chevron";
  chevron.setAttribute("aria-hidden", "true");
  chevron.innerHTML = '<svg viewBox="0 0 12 12"><path d="M3 4.5 6 7.5l3-3"/></svg>';
  const list = document.createElement("select");
  list.setAttribute("aria-label", `Choose the ${name} from a list`);
  const keys = document.createElement("optgroup");
  keys.label = "Keys";
  const mice = document.createElement("optgroup");
  mice.label = "Mouse buttons";
  for (const k of state.key_options) {
    if (k.value.startsWith("mouse-")) {
      // Mouse buttons can only be the dictation key.
      if (id === "hk-dictation") mice.append(new Option(k.label, k.value));
    } else keys.append(new Option(k.label, k.value));
  }
  const other = document.createElement("optgroup");
  other.label = "—";
  other.append(new Option(`Reset to ${keyLabel(defaultKey(id))}`, "default"));
  list.append(keys, ...(mice.children.length ? [mice] : []), other);
  list.value = value;
  list.onchange = () => setKey(id, list.value === "default" ? defaultKey(id) : list.value);
  more.append(chevron, list);

  picker.replaceChildren(main, more);
}

/** Chooses a key for one picker and saves, unless the other one has it. */
function setKey(id, value) {
  const otherId = id === "hk-dictation" ? "hk-instruction" : "hk-dictation";
  if ($(otherId).dataset.value === value) {
    renderKeyPicker(id, $(id).dataset.value);
    showHotkeyWarning(`${keyLabel(value)} is already the ${$(otherId).dataset.name}. Choose another key, or change that one first.`);
    return;
  }
  renderKeyPicker(id, value);
  renderHotkeyTip();
  save();
}

let capture = null; // { picker, onKey, onMouse, timer }

function startCapture(picker) {
  if (capture) stopCapture();
  invoke("pause_hotkeys", { paused: true }).catch(() => {});
  picker.classList.add("recording");
  const main = picker.querySelector(".kp-main");
  main.setAttribute("aria-pressed", "true");
  const mouse = picker.id === "hk-dictation";
  main.innerHTML = `<span class="kp-dot" aria-hidden="true"></span><span>Press a key…</span><span class="kp-hint">Esc cancels</span>`;
  main.setAttribute("aria-label", `Press the key you want${mouse ? ", or a side mouse button" : ""}. Escape cancels.`);

  const onKey = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "Escape") return stopCapture();
    const key = CODE_TO_KEY[e.code];
    if (!key) {
      showHotkeyWarning("Use a modifier key: Option, Command, Control or Shift. Fn and mouse buttons are in the ▾ list.");
      return;
    }
    finishCapture(picker, key);
  };
  const onMouse = (e) => {
    const key = MOUSE_TO_KEY[e.button];
    if (key && mouse) {
      // Keep Back/Forward from navigating the page.
      e.preventDefault();
      if (e.type === "mousedown") finishCapture(picker, key);
    } else if (e.type === "mousedown" && e.button === 0 && !picker.contains(e.target)) {
      stopCapture();
    }
  };
  window.addEventListener("keydown", onKey, true);
  for (const t of ["mousedown", "mouseup", "auxclick"]) window.addEventListener(t, onMouse, true);
  // Same limit as the backend's pause.
  capture = { picker, onKey, onMouse, timer: setTimeout(stopCapture, 14000) };
}

function finishCapture(picker, key) {
  stopCapture();
  setKey(picker.id, key);
  picker.querySelector(".kp-main")?.focus();
}

function stopCapture() {
  if (!capture) return;
  const { picker, onKey, onMouse, timer } = capture;
  window.removeEventListener("keydown", onKey, true);
  for (const t of ["mousedown", "mouseup", "auxclick"]) window.removeEventListener(t, onMouse, true);
  clearTimeout(timer);
  capture = null;
  invoke("pause_hotkeys", { paused: false }).catch(() => {});
  renderKeyPicker(picker.id, picker.dataset.value);
}

window.addEventListener("blur", stopCapture);

/** A warning under the hotkeys, until the keys change again. */
function showHotkeyWarning(text) {
  $("hk-tip").textContent = text;
  $("hk-tip").classList.add("warn-text");
  $("hk-tip-row").hidden = false;
}

/** Explains trade-offs of the chosen keys, and shows what the dictation key
 *  does. */
function renderHotkeyTip() {
  const d = $("hk-dictation").dataset.value;
  const i = $("hk-instruction").dataset.value;
  const mac = state.platform === "macos";
  let tip = "";
  if (d === i) tip = "The dictation and instruction keys must be different.";
  else if (d.endsWith("shift")) tip = `Shift also types capital letters, so holding it to dictate can get in the way. ${mac ? "A right-hand Command, Option or Control key" : "Right Ctrl"} usually works better.`;
  else if (d.startsWith("left-")) tip = "Left-side modifier keys are in many shortcuts. That's fine (shortcuts cancel dictation silently), but a right-hand key is less likely to clash.";
  else if (d === "right-alt" && mac) tip = "Tip: Option also types special characters (é, ü, ©). If you use those a lot, Right Command is a good alternative.";
  else if (d === "right-alt" && !mac) tip = "Right Alt is AltGr on many keyboard layouts and types special characters. Right Ctrl is usually a safer choice.";
  else if (d === "fn") tip = "To use Fn, set System Settings → Keyboard → “Press 🌐 key to” → Do Nothing.";
  $("hk-tip").textContent = tip;
  $("hk-tip").classList.toggle("warn-text", d === i);
  $("hk-tip-row").hidden = !tip;
  renderHotkeyLegend(d);
}

function renderHotkeyLegend(d) {
  const k = keyShort(d);
  const mouse = d.startsWith("mouse-");
  const letter = (state.settings.hotkeys.hands_free || "h").toUpperCase();
  const items = [
    [`Hold ${k}`, "Talk", "let go to type it"],
    [mouse ? `${k} ×2` : `${k} + ${letter}`, "Hands-free", mouse ? "click again to finish" : `then let go; tap ${k} to finish`],
    [`${k} + ↩`, "Send", "presses Return after"],
    ["Esc", "Cancel", ""],
    [`${k} ${keyShort($("hk-instruction").dataset.value)}`, "Command", "tap the instruction key first, then say it, e.g. “delete that”"],
  ];
  $("hk-legend").replaceChildren(
    ...items.map(([keys, what, how]) => {
      const item = document.createElement("div");
      const cap = document.createElement("span");
      cap.className = "kbd";
      cap.textContent = keys;
      const text = document.createElement("span");
      const b = document.createElement("b");
      b.textContent = what;
      text.append(b, how ? `, ${how}` : "");
      item.append(cap, text);
      return item;
    })
  );
}

// ---------------------------------------------------------------- launch at login

// Turning it on downloads the speaker model (once); if that fails, it goes
// back off.
$("mt-speakers").addEventListener("change", async (e) => {
  if (!e.target.checked) return;
  const note = $("mt-speakers-note");
  const before = note.textContent;
  const stop = await listen("model-download", (ev) => {
    if (ev.payload.id !== "sortformer" || !ev.payload.total) return;
    note.textContent = `Downloading the speaker model… ${Math.floor((ev.payload.done / ev.payload.total) * 100)}%`;
  });
  try {
    await invoke("download_model", { id: "sortformer" });
  } catch (err) {
    e.target.checked = false;
    e.target.dispatchEvent(new Event("change", { bubbles: true }));
    toast(`Couldn't download the speaker model: ${err}`, true);
  } finally {
    stop();
    note.textContent = before;
  }
});

// Turning it on downloads the embedding model, then reads the notes.
$("nt-meaning").addEventListener("change", async (e) => {
  if (!e.target.checked) return;
  const note = $("nt-meaning-note");
  const stop = await listen("model-download", (ev) => {
    if (ev.payload.id !== "qwen3-embedding" || !ev.payload.total) return;
    note.textContent = `Downloading the model… ${Math.floor((ev.payload.done / ev.payload.total) * 100)}%`;
  });
  try {
    await invoke("download_model", { id: "qwen3-embedding" });
  } catch (err) {
    e.target.checked = false;
    e.target.dispatchEvent(new Event("change", { bubbles: true }));
    toast(`Couldn't download the model: ${err}`, true);
    note.textContent = MEANING_NOTE;
    return;
  } finally {
    stop();
  }
  // Saved by now, so the notes are read.
  setTimeout(showMeaningProgress, 500);
});

// While the notes are being read for meaning, how far it's got.
async function showMeaningProgress() {
  if (meaningShown) return;
  meaningShown = true;
  const note = $("nt-meaning-note");
  while ($("nt-meaning").checked) {
    const [done, total] = await invoke("notes_meaning_progress").catch(() => [0, 0]);
    if (!total || done >= total) break;
    note.textContent = `Reading your notes: ${done} of ${total}…`;
    await new Promise((r) => setTimeout(r, 2000));
  }
  note.textContent = MEANING_NOTE;
  meaningShown = false;
}

// Turning it on asks macOS for the calendar; refused, it goes back off.
$("mt-calendar").addEventListener("change", async (e) => {
  if (!e.target.checked) return;
  const allowed = await invoke("calendar_access").catch(() => false);
  if (allowed) return;
  e.target.checked = false;
  e.target.dispatchEvent(new Event("change", { bubbles: true }));
  toast("macOS didn't allow it. You can allow Splashdown in System Settings → Privacy & Security → Calendars.", true);
});

$("gen-login").addEventListener("change", async (e) => {
  try {
    await invoke("set_launch_at_login", { enabled: e.target.checked });
    toast(e.target.checked ? "Will start at login" : "Won't start at login");
  } catch (err) {
    toast(String(err), true);
    e.target.checked = !e.target.checked;
  }
});
invoke("get_launch_at_login").then((on) => ($("gen-login").checked = !!on)).catch(() => {});

// ---------------------------------------------------------------- microphone

/** The microphones last listed: `{ devices, automatic }`. */
let micList = { devices: [], automatic: null };

async function loadMicrophones() {
  micList = await invoke("list_microphones").catch(() => ({ devices: [], automatic: null }));
  renderMicrophones();
}

/** The microphone and, once one is chosen, the one to use when it's not
 *  connected. */
function renderMicrophones() {
  // At startup the first tab can open before the state arrives; startup
  // calls this again once it has.
  if (!state) return;
  const g = state.settings.general;
  const tag = (m) => [m.built_in && "built-in", m.bluetooth && "Bluetooth"].filter(Boolean).join(", ");
  const options = (skip) => [
    new Option(`Automatic${micList.automatic ? ` (${micList.automatic})` : ""}`, ""),
    ...micList.devices
      .filter((m) => m.name !== skip)
      .map((m) => new Option(tag(m) ? `${m.name} — ${tag(m)}` : m.name, m.name)),
  ];
  const missing = (name) => new Option(`${name} — not connected`, name);
  const connected = (name) => micList.devices.some((m) => m.name === name);
  const sel = $("gen-mic");
  if (document.activeElement !== sel) {
    const list = options(null);
    if (g.microphone && !connected(g.microphone)) list.push(missing(g.microphone));
    sel.replaceChildren(...list);
    sel.value = g.microphone ?? "";
  }
  const backup = $("gen-mic-backup");
  $("gen-mic-backup-row").hidden = !g.microphone;
  if (document.activeElement !== backup) {
    const list = options(g.microphone);
    if (g.microphone_backup && !connected(g.microphone_backup)) list.push(missing(g.microphone_backup));
    backup.replaceChildren(...list);
    backup.value = g.microphone_backup ?? "";
  }
}

// ---------------------------------------------------------------- sound volume

// Save and let the user hear the new level when they let go of the slider.
$("gen-volume").addEventListener("change", (e) => {
  save();
  invoke("preview_sound", { volume: Number(e.target.value) / 100 }).catch(() => {});
});

// ---------------------------------------------------------------- learned words

/** The words learned from corrections, each with ✕ to forget it (it's
 *  removed and never learned again). */
function showLearned(words) {
  const host = $("vocab-learned");
  host.hidden = words.length === 0;
  host.replaceChildren(
    document.createTextNode("Learned from your corrections:"),
    ...words.map((w) => {
      const chip = document.createElement("span");
      chip.className = "chip";
      chip.append(w);
      const forget = document.createElement("button");
      forget.className = "no";
      forget.textContent = "✕";
      forget.title = `Forget ${w}`;
      forget.setAttribute("aria-label", `Forget ${w}`);
      // Settings reloads once it's saved (settings-reloaded).
      forget.onclick = () => invoke("forget_learned_word", { word: w }).catch((e) => toast(String(e), true));
      chip.append(forget);
      return chip;
    })
  );
}
