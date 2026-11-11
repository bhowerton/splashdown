"use strict";

/** Settings → General → Updates: "" (tell me), "install" (by themselves)
 *  or "off" (don't check), from the two settings behind it. */
function updatesChoice(general) {
  if (general.check_for_updates === false) return "off";
  return general.install_updates === true ? "install" : "";
}

/** What the choice of updates means, under it. */
function describeUpdates() {
  $("gen-updates-what").textContent = {
    "": "Looks for a new version at launch and once a day. It installs only when you click; see About.",
    install: "A new version installs by itself once it has been out two days, at a moment when Splashdown has been idle for ten minutes: nothing recording, no window open. It restarts, and the menu bar says what it updated to.",
    off: "Splashdown never looks for a new version. Check by hand in About.",
  }[$("gen-updates").value];
}

// General, Notes and Meetings tabs: hotkey pickers and their legend, launch at login, updates,
// microphone, live preview, sounds. Also the Notes and Meetings sections
// (watched folders, people known by voice, the calendar), and the Words &
// language tab's name, languages, translate targets, vocabulary, learned
// words and writing style (its snippets live in snippets.js).

/** The ask letter when none is set: the first of A, K, J, Q that hands-free
 *  and notes don't use (as `Hotkeys::ask_key`). */
function askDefault(hotkeys) {
  const hands = (hotkeys.hands_free || "h").toLowerCase();
  const note = (hotkeys.note || (hands === "n" ? "m" : "n")).toLowerCase();
  return ["a", "k", "j", "q"].find((c) => c !== hands && c !== note) ?? "a";
}

/** The translate letter when none is chosen: T, or the next one free
 *  (matches HotkeySettings::translate_key). */
function translateDefault(hotkeys) {
  const hands = (hotkeys.hands_free || "h").toLowerCase();
  const note = (hotkeys.note || (hands === "n" ? "m" : "n")).toLowerCase();
  const ask = (hotkeys.ask || askDefault(hotkeys)).toLowerCase();
  const compose = (hotkeys.compose || "").toLowerCase();
  const taken = [hands, note, ask, compose];
  return ["t", "l", "y"].find((c) => !taken.includes(c)) ?? "t";
}

/** Fallback for a backend that doesn't send `defaults.sound_volume` yet.
 *  Matches splashdown_config's DEFAULT_SOUND_VOLUME. */
const DEFAULT_SOUND_VOLUME = 0.12;
// What "Find notes by meaning" says when it's not busy, and whether its
// progress is being shown.
const MEANING_NOTE = $("nt-meaning-note").textContent;
let meaningShown = false;

/** The invite link for the list in use, if a teammate's computer could
 *  follow one. */
let sharedInvite = null;

/** "Your team's words": the file in use and what it gave, or why it
 *  isn't being used, and the buttons that fit. */
async function renderSharedWords(status) {
  try {
    status ??= await invoke("shared_words_status");
  } catch {
    return;
  }
  const note = $("shared-words-status");
  const count = (n, one) => `${n} ${one}${n === 1 ? "" : "s"}`;
  const using = !!status.file;
  sharedInvite = status.invite ?? null;
  note.hidden = !using;
  note.classList.toggle("warn-text", !!status.problem);
  if (using) {
    const name = status.file.split(/[\\/]/).pop();
    note.title = status.file;
    note.textContent = status.problem
      ? `${name} isn't being used: ${status.problem}.`
      : `${name}: ${count(status.words, "word")} and ${count(status.corrections, "correction")} in use.` +
        (status.left_out ? ` ${count(status.left_out, "line")} left out (a repeat, longer than a name or term, or past the first 300).` : "") +
        (sharedInvite ? "" : " It isn't in a synced folder Splashdown knows, so there's no invite link: teammates choose the file themselves.");
  }
  $("shared-words-invite").hidden = !sharedInvite;
  $("shared-words-edit").hidden = !using;
  $("shared-words-start").hidden = using;
  $("shared-words-join").hidden = using;
  $("shared-words-choose").textContent = using ? "Choose another…" : "Choose a file…";
  $("shared-words-stop").hidden = !using;
}

/** Puts the invite link on the clipboard; whether it did. */
async function copySharedInvite() {
  if (!sharedInvite) return false;
  try {
    await navigator.clipboard.writeText(sharedInvite);
    return true;
  } catch {
    return false;
  }
}

async function setSharedWords(file) {
  try {
    const status = await invoke("shared_words_set", { file });
    renderSharedWords(status);
    toast(file ? `Using ${status.words + status.corrections} of your team's words and corrections` : "No longer using your team's words");
  } catch (e) {
    toast(String(e), true);
  }
}

$("shared-words-choose").addEventListener("click", async () => {
  let picked;
  try {
    picked = await invoke("shared_words_pick");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  if (picked) setSharedWords(picked);
});
$("shared-words-stop").addEventListener("click", () => setSharedWords(null));

// Starts a list in a folder the team shares, and copies the link that
// lets the others join.
$("shared-words-start").addEventListener("click", async () => {
  let folder;
  try {
    folder = await invoke("shared_words_pick_folder");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  if (!folder) return;
  const mine = state.settings.general.vocabulary.length + state.settings.replacements.length;
  const withMine =
    mine > 0 &&
    (await askConfirm("Start it with your own words?", {
      action: "Add my words",
      cancel: "Start empty",
      danger: false,
      detail: "Your names, terms and corrections become the team's first entries, for everyone who joins to see and use. They stay in your own vocabulary too.",
    }));
  try {
    const started = await invoke("shared_words_start", { folder, withMine });
    await renderSharedWords(started.status);
    const copied = await copySharedInvite();
    toast(
      (started.existed ? "That folder already had a team list: using it." : "Team list started.") +
        (copied ? " The invite link is copied: paste it to your team." : ""),
    );
  } catch (e) {
    toast(String(e), true);
  }
});

$("shared-words-invite").addEventListener("click", async () => {
  toast((await copySharedInvite()) ? "Invite link copied: paste it to your team" : "Couldn't copy the link", !sharedInvite);
});

$("shared-words-edit").addEventListener("click", () => call("shared_words_edit"));

$("shared-words-join").addEventListener("click", async () => {
  const link = await askText("Join your team's words", {
    detail: "Paste the invite link a teammate sent you.",
    placeholder: "https://splashdown.app/team/#…",
    action: "Join",
  });
  if (!link) return;
  try {
    const status = await invoke("shared_words_join", { link });
    renderSharedWords(status);
    toast(`Using ${status.words + status.corrections} of your team's words and corrections`);
  } catch (e) {
    toast(String(e), true);
  }
});

function renderGeneral() {
  renderSharedWords();
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
  setValue($("hk-translate"), (s.hotkeys.translate || translateDefault(s.hotkeys)).toUpperCase());
  $("gen-compose-panel").checked = s.general.compose_panel === true;
  $("gen-compose-style").checked = s.general.compose_style !== false;
  const notes = s.notes ?? {};
  setValue($("nt-capture-to"), notes.capture_to ?? "inbox");
  $("nt-daily-row").hidden = (notes.capture_to ?? "inbox") !== "daily";
  setValue($("nt-daily-folder"), notes.daily_folder ?? "");
  setValue($("nt-daily-format"), notes.daily_format ?? "");
  $("nt-keep-audio").checked = !!notes.keep_audio;
  renderWatched(notes.watched ?? []);
  renderNotesPlace();
  setValue($("mt-summary"), notes.meeting_summary ?? "");
  $("mt-keep-audio").checked = !!notes.keep_meeting_audio;
  $("mt-stop-quiet").checked = notes.stop_when_quiet ?? true;
  $("mt-offer").checked = notes.offer_on_calls ?? true;
  // Apps the offer's "Don't offer for Zoom" left out.
  const skipped = notes.offer_skip ?? [];
  $("mt-skip-row").hidden = skipped.length === 0;
  $("mt-skip").textContent = skipped.join(", ");
  $("mt-calendar").checked = !!notes.meeting_calendar;
  $("mt-calendar-offer").checked = !!notes.offer_on_calendar;
  $("mt-speakers").checked = !!notes.meeting_speakers;
  // Not on Windows yet: the voice model's runtime isn't installed there.
  $("mt-remember-row").hidden = state.platform === "windows";
  $("mt-remember").checked = !!notes.remember_speakers;
  renderVoices();
  $("nt-meaning").checked = !!notes.ask_by_meaning;
  if (notes.ask_by_meaning) showMeaningProgress();
  // EventKit is the Mac's; there's no calendar to read on Windows yet.
  $("mt-calendar-row").hidden = state.platform !== "macos";
  $("mt-calendar-offer-row").hidden = state.platform !== "macos";
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
  renderTranslateTargets(s.general.translate_targets ?? []);
  $("gen-their").checked = !!s.general.reply_in_their_language;
  // It reads the conversation with the screen words, so it can't without.
  const screenOff = s.general.screen_words === false;
  $("gen-their").disabled = screenOff && !s.general.reply_in_their_language;
  $("gen-their-note").classList.toggle("warn-text", screenOff);
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
  $("gen-learn-style").checked = s.general.learn_style !== false;
  loadStyles();
  setValue($("gen-updates"), updatesChoice(s.general));
  describeUpdates();
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

/** Languages ⌃⌥T can translate into, by the English name the AI is told. */
const TRANSLATE_TARGETS = [
  "English", "Spanish", "French", "German", "Italian", "Portuguese", "Dutch", "Swedish", "Norwegian",
  "Danish", "Finnish", "Polish", "Czech", "Ukrainian", "Russian", "Greek", "Turkish", "Arabic", "Hebrew",
  "Hindi", "Bengali", "Indonesian", "Vietnamese", "Thai", "Japanese", "Korean",
  "Chinese (Simplified)", "Chinese (Traditional)",
];
/** Most languages the menu bar lists. */
const MOST_TRANSLATE_TARGETS = 5;

function renderTranslateTargets(chosen) {
  // Kept as chosen, even one this list no longer has.
  const names = [...TRANSLATE_TARGETS, ...chosen.filter((c) => !TRANSLATE_TARGETS.includes(c))];
  $("gen-translate").replaceChildren(
    ...names.map((name) => {
      const label = document.createElement("label");
      label.className = "lang-chip";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.value = name;
      box.checked = chosen.includes(name);
      box.onchange = () => {
        limitTranslateTargets();
        save();
      };
      label.append(box, document.createTextNode(name));
      return label;
    })
  );
  limitTranslateTargets();
}

/** Five ticked: the rest can't be ticked until one is cleared. */
function limitTranslateTargets() {
  const boxes = [...document.querySelectorAll("#gen-translate input")];
  const full = boxes.filter((b) => b.checked).length >= MOST_TRANSLATE_TARGETS;
  for (const box of boxes) box.disabled = full && !box.checked;
}

/** The ticked languages to translate into, for collect(). */
function translateTargets() {
  return [...document.querySelectorAll("#gen-translate input:checked")].map((b) => b.value);
}

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
      showHotkeyWarning(state.platform === "macos"
        ? "Use a modifier key: Option, Command, Control or Shift. Fn and mouse buttons are in the ▾ list."
        : "Use a modifier key: Ctrl, Alt, Shift or Windows.");
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
  // Recognizing people can't work without it.
  if (!e.target.checked) {
    if ($("mt-remember").checked) {
      $("mt-remember").checked = false;
      $("mt-remember").dispatchEvent(new Event("change", { bubbles: true }));
    }
    return;
  }
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

// Turning either on asks macOS for the calendar; refused, it goes back off.
for (const id of ["mt-calendar", "mt-calendar-offer"]) {
  $(id).addEventListener("change", async (e) => {
    if (!e.target.checked) return;
    const allowed = await invoke("calendar_access").catch(() => false);
    if (allowed) return;
    e.target.checked = false;
    e.target.dispatchEvent(new Event("change", { bubbles: true }));
    toast("macOS didn't allow it. You can allow Splashdown in System Settings → Privacy & Security → Calendars.", true);
  });
}

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

// ---------------------------------------------------------------- writing style

/** The style learned for each kind of app (`style.rs`): an editable
 *  description, saved when it's changed, and how many edited drafts it
 *  came from. Kinds with nothing learned yet are left out. */
async function loadStyles() {
  const host = $("style-kinds");
  const kinds = await invoke("style_profiles").catch(() => []);
  // Not while a description is being edited.
  if (typingTextIn(host)) return;
  const shown = kinds.filter((k) => k.profile || k.edits);
  if (!shown.length) {
    const none = document.createElement("div");
    none.className = "row";
    none.innerHTML = '<span class="lbl"><small>Nothing learned yet. Edit a few drafts Compose writes into a reply, then send them.</small></span>';
    host.replaceChildren(none);
    return;
  }
  host.replaceChildren(
    ...shown.map((k) => {
      const row = document.createElement("div");
      row.className = "row column";
      const lbl = document.createElement("span");
      lbl.className = "lbl";
      const small = document.createElement("small");
      small.textContent = k.profile
        ? `From ${plural(k.edits, "edited draft")}. Change it as you like: the next update starts from your wording.`
        : `${plural(k.edits, "edited draft")} so far; a description is written after a few more.`;
      lbl.append(k.label, small);
      const text = document.createElement("textarea");
      text.rows = 3;
      text.value = k.profile;
      text.placeholder = "Sign off with “Cheers, Ben”.";
      text.setAttribute("aria-label", `${k.label} style`);
      text.addEventListener("change", () =>
        invoke("style_set_profile", { kind: k.kind, profile: text.value })
          .then(() => toast("Saved"))
          .catch((e) => toast(String(e), true))
      );
      row.append(lbl, text);
      return row;
    })
  );
}

$("style-forget").addEventListener("click", async () => {
  const ok = await askConfirm("Forget your writing style?", {
    detail: "Every description and every kept draft is deleted. Compose starts learning again from your next edits.",
    action: "Forget",
  });
  if (!ok) return;
  await call("style_forget");
  loadStyles();
  toast("Forgotten");
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
      forget.onclick = () => call("forget_learned_word", { word: w });
      chip.append(forget);
      return chip;
    })
  );
}

// ---------------------------------------------------------------- watched folders

/** A row per watched folder: where its notes go, subtitles beside the
 *  files, and the files that were there before it was added. */
function renderWatched(folders) {
  // Not while a folder's "notes go in" is being typed.
  if (typingTextIn($("nt-watched"))) return;
  const rows = folders.map((f) => {
    const row = document.createElement("div");
    row.className = "watch-row";
    row.dataset.path = f.path;
    const name = document.createElement("div");
    name.className = "watch-path";
    name.textContent = f.path;
    name.title = f.path;
    const into = document.createElement("input");
    into.type = "text";
    into.className = "watch-into";
    into.placeholder = "Top of your notes";
    into.setAttribute("aria-label", "Notes folder for its transcripts");
    into.spellcheck = false;
    into.value = f.into ?? "";
    into.addEventListener("change", () => save());
    const intoLabel = document.createElement("label");
    intoLabel.className = "watch-opt";
    intoLabel.append("Notes go to ", into);
    const subs = document.createElement("input");
    subs.type = "checkbox";
    subs.className = "watch-subs";
    subs.checked = !!f.subtitles;
    subs.addEventListener("change", () => save());
    const subsLabel = document.createElement("label");
    subsLabel.className = "watch-opt";
    subsLabel.append(subs, " Save subtitles (.srt) next to each file");
    const waiting = document.createElement("button");
    waiting.type = "button";
    waiting.className = "link watch-waiting";
    waiting.hidden = true;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "link danger";
    remove.textContent = "Stop watching";
    remove.addEventListener("click", () => {
      row.remove();
      save();
    });
    const acts = document.createElement("div");
    acts.className = "watch-acts";
    acts.append(waiting, remove);
    row.append(name, intoLabel, subsLabel, acts);
    showWaiting(f.path, waiting);
    return row;
  });
  $("nt-watched").replaceChildren(...rows);
}

/** "Transcribe the 12 files already here", when there are some. */
async function showWaiting(path, button) {
  let count = 0;
  try {
    count = await invoke("notes_watch_waiting", { path });
  } catch {
    return;
  }
  if (!count) return;
  button.textContent = count === 1 ? "Transcribe the file already here" : `Transcribe the ${count} files already here`;
  button.hidden = false;
  button.onclick = async () => {
    try {
      const n = await invoke("notes_watch_take_waiting", { path });
      button.hidden = true;
      toast(`Transcribing ${plural(n, "file")}: see Notes`);
    } catch (e) {
      toast(String(e), true);
    }
  };
}

/** The watched folders as the rows show them, for saving. */
function watchedFolders() {
  return [...document.querySelectorAll("#nt-watched .watch-row")].map((row) => ({
    path: row.dataset.path,
    into: row.querySelector(".watch-into").value.trim(),
    subtitles: row.querySelector(".watch-subs").checked,
  }));
}

// ---------------------------------------------------------------- where notes are kept

/** The notes folder, and a button for each sync service on this computer
 *  (and for this computer, when they're in one) that moves them there.
 *  Asking doesn't create the folder. */
async function renderNotesPlace() {
  let folder, places;
  try {
    [folder, places] = await Promise.all([invoke("notes_folder"), invoke("notes_cloud_places")]);
  } catch {
    return;
  }
  const note = $("nt-folder-note");
  note.textContent = folder.synced
    ? `${folder.label}, in ${folder.synced}.`
    : `${folder.label}, on this computer.`;
  note.title = folder.path;
  const inside = (p) => folder.path === p.folder || folder.path.startsWith(p.folder.replace(/Splashdown Notes$/, ""));
  const options = places.cloud.filter((p) => !inside(p));
  // Back to Documents, once they're kept in a sync service.
  if (folder.synced && places.local && places.local.folder !== folder.path) options.push(places.local);
  const list = $("nt-places");
  list.replaceChildren(...options.map((p) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "btn";
    b.textContent = p.account ? `${p.service} (${p.account})` : p.service;
    b.title = p.folder;
    b.addEventListener("click", () => moveNotesTo(p));
    return b;
  }));
  $("nt-places-row").hidden = options.length === 0;
  renderPhone();
}

// What "From your iPhone" says, before what's known about this computer.
const PHONE_NOTE = $("nt-phone-note").textContent;

/** "From your iPhone", with whether this computer can get the notes. */
async function renderPhone() {
  let phone;
  try {
    phone = await invoke("notes_phone");
  } catch {
    return;
  }
  const note = $("nt-phone-note");
  note.textContent = PHONE_NOTE;
  note.classList.toggle("warn-text", !phone.icloud);
  if (!phone.icloud) {
    note.textContent += state.platform === "windows"
      ? " This computer doesn't have iCloud Drive yet: install iCloud for Windows from the Microsoft Store and turn on iCloud Drive."
      : " iCloud Drive is off on this Mac: turn it on in System Settings → your name → iCloud.";
  } else if (phone.used) {
    note.textContent += " Your iPhone's notes are arriving here.";
  }
}

/** Moves the notes to `place` (a sync service, or this computer). */
async function moveNotesTo(place) {
  const where = place.service === "this computer" ? "this computer" : place.service;
  const sure = await askConfirm(`Keep your notes in ${where}?`, {
    action: "Move",
    danger: false,
    detail: place.service === "this computer"
      ? `They move to ${place.label}. Your other devices stop getting new ones.`
      : `They move to ${place.label}, and ${place.service} keeps a copy. A note that's there already is kept: yours gets a number.`,
  });
  if (!sure) return;
  try {
    const view = await invoke("notes_set_folder", { folder: place.folder, moveNotesToo: true });
    toast(`Notes are now in ${view.folder.label}`);
  } catch (e) {
    toast(String(e), true);
  }
  renderNotesPlace();
  renderNotesFolder();
}

$("nt-folder-choose").addEventListener("click", async () => {
  let picked;
  try {
    picked = await invoke("notes_pick_folder");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  if (!picked) return;
  const moveToo = await askConfirm("Move your notes there too?", {
    action: "Move",
    danger: false,
    detail: "Otherwise they stay where they are, and Notes shows the new folder's notes.",
  });
  try {
    const view = await invoke("notes_set_folder", { folder: picked, moveNotesToo: moveToo });
    toast(`Notes are now in ${view.folder.label}`);
  } catch (e) {
    toast(String(e), true);
  }
  renderNotesPlace();
  renderNotesFolder();
});

$("nt-phone-get").addEventListener("click", () => call("notes_phone_page"));

$("nt-watch-add").addEventListener("click", async () => {
  let path;
  try {
    path = await invoke("notes_watch_pick");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  if (!path) return;
  const folders = watchedFolders();
  if (folders.some((f) => f.path === path)) {
    toast("That folder is watched already.");
    return;
  }
  const settings = collect();
  settings.notes.watched = [...folders, { path, into: "", subtitles: false }];
  save(settings);
});

// ---------------------------------------------------------------- people known by voice

/** The people saved by voice, each with Forget. */
async function renderVoices() {
  let voices = [];
  try {
    voices = await invoke("voices_list");
  } catch {
    voices = [];
  }
  $("mt-voices-row").hidden = voices.length === 0;
  const list = $("mt-voices");
  list.replaceChildren(
    ...voices.flatMap((v, i) => {
      const forget = document.createElement("button");
      forget.type = "button";
      forget.className = "link";
      forget.textContent = "forget";
      forget.title = `Forget ${v.name}'s voice`;
      forget.addEventListener("click", async () => {
        await call("voices_forget", { name: v.name });
        renderVoices();
      });
      const meetings = plural(v.meetings, "meeting");
      return [...(i ? ["; "] : []), `${v.name} (${meetings}, `, forget, ")"];
    }),
  );
}

$("mt-voices-forget").addEventListener("click", async () => {
  try {
    await invoke("voices_forget", { name: null });
    toast("Every saved voice is forgotten");
  } catch (e) {
    toast(String(e), true);
  }
  renderVoices();
});

// Turning it on turns on telling people apart too (it names the speakers
// that finds), and downloads both models (once); if that fails, it goes
// back off.
$("mt-remember").addEventListener("change", async (e) => {
  if (!e.target.checked) return;
  if (!$("mt-speakers").checked) {
    $("mt-speakers").checked = true;
    $("mt-speakers").dispatchEvent(new Event("change", { bubbles: true }));
  }
  const note = $("mt-remember-note");
  const before = note.textContent;
  const stop = await listen("model-download", (ev) => {
    if (ev.payload.id !== "titanet-small" || !ev.payload.total) return;
    note.textContent = `Downloading the voice model… ${Math.floor((ev.payload.done / ev.payload.total) * 100)}%`;
  });
  try {
    await invoke("download_model", { id: "titanet-small" });
  } catch (err) {
    e.target.checked = false;
    e.target.dispatchEvent(new Event("change", { bubbles: true }));
    toast(`Couldn't download the voice model: ${err}`, true);
  } finally {
    stop();
    note.textContent = before;
  }
});

$("mt-skip-clear").addEventListener("click", () => {
  const settings = collect();
  settings.notes.offer_skip = [];
  save(settings);
});
