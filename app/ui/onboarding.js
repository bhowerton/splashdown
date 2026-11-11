"use strict";

// First-run setup guide (needs common.js). Opened as a plain file in a
// browser it uses the sample data in mockInvoke, for design work.
//
// Written for people who've never heard of an AI model. What matters is
// talking instead of typing: speech to text comes first (about half a
// gigabyte), and the AI that cleans up is optional and arrives in the
// background while they try things. One obvious choice (free and private,
// on this computer); API keys for those who have one.

const LAST = 5;
let step = 1;
let state = null;
let choice = "local"; // "local" | "openrouter" | "openai" | "anthropic"
let plan = null; // what setting up on this computer would download
let setup = null; // how it's going, once started
let tryIndex = 0; // which of tries() is being tried (its length: all done)
let trySince = 0; // when it began, so only what's done after counts
let tryState = []; // "done" or "skipped", by index
let tryTicked = false; // the current one worked; moving on in a moment
let actionsAnswer = null; // "yes" or "no" once asked about voice actions
let slowAnswered = false; // chose what to do about a slow built-in model
let wasWaiting = false; // step 4 was waiting for the downloads
let voiceHere = false; // Phonon, the natural voice, is downloaded
let voiceAsked = false; // its download has been started

// ---------------------------------------------------------------- navigation

/** Windows asks for no permissions: that step is skipped there, unless
 *  the microphone is turned off for desktop apps (then the guide says how
 *  to turn it on, or practice would wait forever). */
function skipped(n) {
  return n === 3 && state?.platform !== "macos" && state?.permissions?.microphone !== "denied";
}

function go(n) {
  const back = n < step;
  step = Math.max(1, Math.min(LAST, n));
  if (skipped(step)) step += back ? -1 : 1;
  document.querySelectorAll(".ob-step").forEach((s) => (s.hidden = Number(s.dataset.step) !== step));
  document.querySelectorAll(".steps i").forEach((i, idx) => i.classList.toggle("on", idx < step));
  const shown = [1, 2, 3, 4, 5].filter((n) => !skipped(n));
  $("step-of").textContent = `Step ${shown.indexOf(step) + 1} of ${shown.length}`;
  $("back").hidden = step === 1;
  if (step === 4) startPractice();
  renderSetup();
  updateFooter();
}

function updateFooter() {
  const next = $("next");
  const note = $("foot-note");
  note.textContent = "";
  next.disabled = false;
  next.textContent = { 1: "Get started", 5: "Finish" }[step] ?? "Continue";
  if (step === 1) note.textContent = "Takes about 2 minutes";
  if (step === 2) {
    next.disabled = !choiceReady();
    if (KEY_CHOICES[choice] && next.disabled) note.textContent = "Paste your key to continue";
  }
  if (step === 3 && state && !state.permissions.accessibility) {
    next.disabled = true;
    note.textContent = "Accessibility is needed for the dictation key";
  }
  // While getting ready, there's nothing to move on to.
  if (step === 4 && (waiting() || tooSlow())) {
    next.disabled = true;
    next.textContent = "Continue";
    next.classList.add("primary");
    note.textContent = waiting() ? "Speech to text needs to finish downloading first" : "";
    return;
  }
  // While trying things, moving on is a quiet choice, not the main one.
  const skipping = step === 4 && tryIndex < tries().length;
  next.classList.toggle("primary", !skipping);
  if (skipping) next.textContent = "Skip the rest";
}

$("back").onclick = () => go(step - 1);
$("next").onclick = async () => {
  if (step === 2 && !(await applyChoice())) return;
  if (step === LAST) return finish();
  go(step + 1);
};

// ---------------------------------------------------------------- step 2: how it should work

const KEY_CHOICES = {
  openrouter: { label: "OpenRouter", site: "openrouter.ai/keys" },
  openai: { label: "OpenAI", site: "platform.openai.com/api-keys" },
  anthropic: { label: "Anthropic", site: "platform.claude.com/settings/keys" },
};

function existingKey() {
  return state.keys.find((k) => k.source && KEY_CHOICES[k.name]);
}

/** What each key does, in one line. */
const KEY_NOTES = {
  openrouter: "One key for many models: the best cleanup and formatting, a few cents a day even with heavy use.",
  openai: "Speech to text and cleanup with your OpenAI account.",
  anthropic: "Claude cleans up your words; speech to text runs on this computer (a 0.5 GB download).",
};

/** Download the AI that cleans up, with free and private. */
function aiOn() {
  return $("ai-on").checked;
}

function choiceReady() {
  if (choice === "local") return true;
  if (KEY_CHOICES[choice]) return state.keys.some((k) => k.name === choice && k.source);
  return false;
}

function gb(bytes) {
  return (bytes / 1e9).toFixed(1);
}

/** What free and private downloads, in plain words: speech to text, and
 *  the AI if it's wanted. */
function describePlan() {
  const warning = $("space-warning");
  warning.hidden = true;
  if (!plan) return;
  const speechMb = plan.speech_mb ?? plan.download_mb;
  const aiMb = plan.ai_mb ?? 0;
  const mb = speechMb + (aiOn() ? aiMb : 0);
  // What has to come before the first dictation; the AI's size is on its line.
  $("local-size").textContent = mb === 0 ? "Ready" : speechMb === 0 ? "Ready to dictate" : `${gb(speechMb * 1e6)} GB to start`;
  const opt = $("ai-opt-text");
  opt.replaceChildren();
  const b = document.createElement("b");
  if (plan.cleanup === "apple") {
    b.textContent = "Also use Apple Intelligence";
    opt.append(b, ", built into your Mac, to remove the “um”s, rewrite, answer questions and read text back. Only the natural voice to download (68 MB).");
  } else if (plan.cleanup === "none") {
    $("ai-opt").hidden = true;
  } else {
    b.textContent = "Also download the AI";
    // With the natural voice it reads back in (68 MB).
    const size = aiMb ? `${gb((aiMb + 68) * 1e6)} GB` : "already here";
    opt.append(b, ` that removes the “um”s, rewrites, answers questions and reads text back to you (${size}). It arrives while you try things; you can dictate before it's done.`);
  }
  if (plan.too_little_space && mb > 0) {
    warning.textContent = `This computer has ${gb(plan.free_mb * 1e6)} GB of free space, and Splashdown needs about ${gb(mb * 1e6 + 1e9)} GB. Free up some space first (empty the ${state.platform === "macos" ? "Trash" : "Recycle Bin"}, or delete large files you don't need), or use an API key.`;
    warning.hidden = choice !== "local";
  }
}

$("ai-on").addEventListener("change", describePlan);

function keyRow(name) {
  const info = KEY_CHOICES[name];
  const key = state.keys.find((k) => k.name === name);
  const row = document.createElement("div");
  row.className = "row";
  const where = state.platform === "macos" ? "It's kept in your Keychain." : "It's kept safely on this computer.";
  row.innerHTML = `<span class="lbl">${info.label} API key<small>Create one at ${info.site}. ${where}</small></span>`;
  if (key?.source) {
    row.append(badge(`Saved · …${key.hint}`, "ok"));
    return row;
  }
  const input = document.createElement("input");
  input.type = "password";
  input.placeholder = "Paste key…";
  input.autocomplete = "off";
  input.setAttribute("aria-label", `${info.label} API key`);
  const save = document.createElement("button");
  save.className = "btn primary";
  save.textContent = "Verify";
  const submit = async () => {
    if (!input.value.trim()) return input.focus();
    save.disabled = true;
    save.textContent = "Checking…";
    try {
      const r = await invoke("save_key", { name, value: input.value });
      state = r.state;
      toast("Key verified");
      renderChoice();
    } catch (e) {
      toast(String(e), true);
      save.disabled = false;
      save.textContent = "Verify";
    }
    updateFooter();
  };
  save.onclick = submit;
  input.onkeydown = (e) => e.key === "Enter" && submit();
  row.append(input, save);
  setTimeout(() => input.focus(), 0);
  return row;
}

function renderChoice() {
  const key = choice !== "local";
  document.querySelectorAll(".card.choice").forEach((c) => {
    const on = (c.dataset.choice === "key") === key;
    c.classList.toggle("sel", on);
    c.setAttribute("aria-checked", String(on));
  });
  $("ai-opt").hidden = key || plan?.cleanup === "none";
  $("key-area").hidden = !key;
  document.querySelectorAll("#key-provider button").forEach((b) => b.setAttribute("aria-checked", String(b.dataset.provider === choice)));
  const detail = $("choice-detail");
  detail.replaceChildren();
  if (KEY_CHOICES[choice]) detail.append(keyRow(choice));
  $("key-note").textContent = KEY_NOTES[choice] ?? "";
  describePlan();
  updateFooter();
}

document.querySelectorAll(".card.choice").forEach((c) =>
  c.addEventListener("click", () => {
    if (c.dataset.choice === "local") choice = "local";
    else if (choice === "local") choice = existingKey()?.name ?? "openrouter";
    renderChoice();
  }),
);
document.querySelectorAll("#key-provider button").forEach((b) =>
  b.addEventListener("click", () => {
    choice = b.dataset.provider;
    renderChoice();
  }),
);

/** Saves the chosen setup and starts any downloads. Returns false to stay
 *  on the step. */
async function applyChoice() {
  $("next").disabled = true;
  const fail = () => {
    $("next").disabled = false;
    return false;
  };
  try {
    if (choice === "local" && aiOn()) {
      // Saved by the setup itself, as each part arrives.
      setup = await invoke("start_local_setup", { speechOnly: false });
    } else if (choice === "local") {
      // Just speech to text: words go in as they're said.
      await saveProviders("auto", "off");
      setup = await invoke("start_local_setup", { speechOnly: true });
    } else if (choice === "anthropic") {
      // Claude only tidies up; the voice is understood on this computer.
      await saveProviders("auto", "anthropic");
      setup = await invoke("start_local_setup", { speechOnly: true });
    } else {
      await saveProviders("auto", "auto");
    }
  } catch (e) {
    toast(String(e), true);
    return fail();
  }
  watchSetup();
  fetchVoice();
  return true;
}

/** Reading back is in the tries whenever the AI is, in Phonon's natural
 *  voice (never the system's, for English). It's small (68 MB), so it's
 *  fetched in the background, after speech to text so it doesn't slow that
 *  down. */
function fetchVoice() {
  if (voiceAsked || voiceHere || !aiPlanned()) return;
  if (setup && !setup.speech_ready) return;
  voiceAsked = true;
  invoke("download_model", { id: "phonon-en" })
    .then(() => {
      voiceHere = true;
      if (step === 4) renderPractice();
    })
    .catch(() => {
      voiceAsked = false;
    });
}

async function saveProviders(speech, ai) {
  const fresh = await invoke("get_state");
  const settings = structuredClone(fresh.settings);
  settings.speech = { provider: speech, model: null };
  settings.ai = { ...settings.ai, provider: ai, model: null };
  state = (await invoke("save_settings", { settings })).state;
}

// ---------------------------------------------------------------- downloads in the background

/** Recent progress, to say how long is left. */
let samples = [];

function watchSetup() {
  samples = [];
  renderSetup();
}

listen("setup-progress", (event) => {
  setup = event.payload;
  fetchVoice();
  samples.push({ t: Date.now(), done: setup.done });
  samples = samples.filter((s) => Date.now() - s.t < 10_000);
  renderSetup();
});

function timeLeft() {
  if (samples.length < 2) return "";
  const first = samples[0];
  const last = samples[samples.length - 1];
  const rate = (last.done - first.done) / ((last.t - first.t) / 1000);
  if (!(rate > 0)) return "";
  const seconds = (setup.total - setup.done) / rate;
  if (seconds < 60) return " · less than a minute left";
  const minutes = Math.round(seconds / 60);
  return ` · about ${plural(minutes, "minute")} left`;
}

/** Can dictation be tried yet? */
function speechReady() {
  return !setup || setup.speech_ready;
}

/** The strip at the top saying how getting ready is going: on step 3, and
 *  on step 4 while the AI is still on its way (step 4 waits for speech to
 *  text on its own, bigger page). */
function renderSetup() {
  const strip = $("getting-ready");
  strip.hidden = !setup || !(step === 3 || (step === 4 && !waiting() && setup.working));
  if (step === 4) renderWait();
  if (step === LAST) renderDone();
  if (!setup) return;
  const text = $("gr-text");
  const bar = $("gr-bar");
  const retry = $("gr-retry");
  retry.hidden = !setup.error;
  $("gr-progress").hidden = !setup.working;
  bar.style.width = `${setup.total ? Math.floor((setup.done / setup.total) * 100) : 0}%`;
  strip.classList.toggle("ok", !setup.working && !setup.error);
  strip.classList.toggle("error", !!setup.error);
  const amount = setup.total ? `${gb(setup.done)} of ${gb(setup.total)} GB` : "";
  const speechTotal = setup.speech_total ?? 0;
  const aiAmount =
    setup.total > speechTotal ? `${gb(Math.max(0, setup.done - speechTotal))} of ${gb(setup.total - speechTotal)} GB` : "";
  if (setup.error) {
    text.textContent = setup.error;
  } else if (!setup.working) {
    text.textContent = "✓ Splashdown is ready.";
  } else if (!setup.speech_ready) {
    text.textContent = `Downloading speech to text… ${amount}${timeLeft()}`;
  } else if (setup.checking) {
    text.textContent = "✓ You can dictate now · checking how fast the AI is here…";
  } else {
    text.textContent = `✓ You can dictate now · the AI: ${aiAmount}${timeLeft()}`;
  }
}

$("gr-retry").onclick = async () => {
  try {
    setup = await invoke("start_local_setup", { speechOnly: choice === "anthropic" });
    watchSetup();
  } catch (e) {
    toast(String(e), true);
  }
};

// ---------------------------------------------------------------- step 3: permissions

function renderPermissions() {
  renderPermissionRows(state.permissions, "", "step-dot");
  // Windows only ever shows this step for the microphone.
  const windows = state.platform !== "macos";
  $("ax-row").hidden = windows;
  $("perm-title").textContent = windows ? "Allow the microphone" : "Two quick permissions";
  if (step === 3) updateFooter();
}

// ---------------------------------------------------------------- step 4: waiting for the downloads

/** Speech to text still downloading (or stopped by an error): nothing can
 *  be tried yet, and the guide waits here. The AI comes in the background. */
function waiting() {
  return !!setup && (!setup.speech_ready || !!setup.error);
}

/** The AI that cleans up is part of this setup (not turned off). */
function aiPlanned() {
  return state.settings.ai?.provider !== "off";
}

/** The AI is here to use: a cloud key, or the local download is done. */
function aiReady() {
  return aiPlanned() && (!setup || !!setup.cleanup_ready || (!setup.working && !setup.error));
}

/** The built-in model turned out too slow, and nothing's been chosen. */
function tooSlow() {
  return (
    !waiting() &&
    setup?.speed === "slow" &&
    !slowAnswered &&
    state.settings.ai?.provider === "local" &&
    !state.settings.ai?.slow_local_ok
  );
}

function secondsOf(ms) {
  const s = ms / 1000;
  return s < 10 ? s.toFixed(1) : String(Math.round(s));
}

function renderWait() {
  const wait = waiting();
  const slow = tooSlow();
  $("wait").hidden = !wait;
  $("slow").hidden = !slow;
  $("try-area").hidden = wait || slow;
  $("try-h1").textContent = wait ? "Getting ready" : slow ? "One thing first" : "Try it";
  if (wait) {
    wasWaiting = true;
    // Waiting for speech to text alone; the AI follows in the background.
    const speechTotal = setup.speech_total || setup.total;
    const pct = speechTotal ? Math.min(1, setup.done / speechTotal) : 0;
    const box = $("wait");
    box.classList.toggle("landed", !!setup.checking);
    // Down from the sky to the water as the downloads go.
    $("w-chute").style.transform = `translateY(${setup.checking ? 40 : -12 + pct * 48}px)`;
    $("wait-progress").classList.toggle("indeterminate", !!setup.checking);
    $("wait-bar").style.width = setup.checking ? "" : `${Math.floor(pct * 100)}%`;
    $("wait-progress").hidden = !!setup.error;
    $("wait-retry").hidden = !setup.error;
    if (setup.error) {
      $("wait-title").textContent = "The download stopped";
      $("wait-sub").textContent = setup.error;
      $("wait-amount").textContent = "";
    } else if (setup.checking) {
      $("wait-title").textContent = "Checking how fast this computer is";
      $("wait-sub").textContent = "Splashdown is cleaning up a test dictation. This takes a few seconds.";
      $("wait-amount").textContent = "";
    } else {
      $("wait-title").textContent = "Downloading speech to text";
      $("wait-sub").textContent = "So Splashdown can understand you. You can try it as soon as it's here.";
      $("wait-amount").textContent = speechTotal
        ? `${Math.floor(pct * 100)}% · ${gb(setup.done)} of ${gb(speechTotal)} GB${timeLeft()}`
        : "Starting…";
    }
  } else if (slow) {
    $("slow-text").textContent = `The AI would take about ${secondsOf(setup.local_ms)} seconds to clean up each dictation here, because it needs a fast processor or graphics chip. A cloud service is much quicker.`;
  } else {
    const note = $("speed-note");
    note.hidden = setup?.speed !== "usable" || state.settings.ai?.provider !== "local";
    if (!note.hidden) {
      note.textContent = `The AI's cleanup takes about ${secondsOf(setup.local_ms)} seconds on this computer. A cloud key makes it quicker: Settings → AI Models.`;
    }
    if (wasWaiting) {
      // Just finished: start the walk-through afresh.
      wasWaiting = false;
      showTry();
    }
    renderPractice();
  }
  updateFooter();
}

$("wait-retry").onclick = () => $("gr-retry").onclick();

/** The settings as they are now (setup may have changed them since the
 *  last poll), to change and save. */
async function freshSettings() {
  try {
    state = await invoke("get_state");
  } catch {
    /* keep what we have */
  }
  return structuredClone(state.settings);
}

/** Saves a change to the settings; true if it worked. */
async function changeSettings(change) {
  const settings = await freshSettings();
  change(settings);
  try {
    state = (await invoke("save_settings", { settings })).state;
    return true;
  } catch (e) {
    toast(String(e), true);
    return false;
  }
}

async function slowChosen(change) {
  if (!(await changeSettings(change))) return;
  slowAnswered = true;
  renderWait();
  showTry();
}

$("slow-key-save").onclick = async () => {
  const input = $("slow-key");
  if (!input.value.trim()) return input.focus();
  const save = $("slow-key-save");
  save.disabled = true;
  save.textContent = "Checking…";
  try {
    state = (await invoke("save_key", { name: "openrouter", value: input.value })).state;
  } catch (e) {
    toast(String(e), true);
    save.disabled = false;
    save.textContent = "Use this key";
    return;
  }
  // Voice still understood here; only the AI moves.
  await slowChosen((s) => {
    s.ai = { ...s.ai, provider: "openrouter", model: null };
  });
  toast("Key verified · the AI now uses OpenRouter");
};
$("slow-key").onkeydown = (e) => e.key === "Enter" && $("slow-key-save").onclick();
$("slow-off").onclick = () =>
  slowChosen((s) => {
    s.ai = { ...s.ai, provider: "off", model: null };
  });
$("slow-keep").onclick = () =>
  slowChosen((s) => {
    s.ai = { ...s.ai, slow_local_ok: true };
  });

// ---------------------------------------------------------------- step 4: try it

// The main things, one at a time in one practice box, each ticked off when
// Splashdown says it worked (`did_since`), and each skippable.

function kbd(text) {
  const k = document.createElement("kbd");
  k.textContent = text;
  return k;
}

function said(text) {
  const em = document.createElement("em");
  em.textContent = `“${text}”`;
  return em;
}

function keys() {
  const letter = (state.settings.hotkeys.ask ?? "a").toUpperCase();
  // The note letter: N, or M when hands-free has N (as `Hotkeys::note_key`).
  const hands = (state.settings.hotkeys.hands_free || "h").toLowerCase();
  const note = (state.settings.hotkeys.note || (hands === "n" ? "m" : "n")).toUpperCase();
  return { dict: dictationKeyLabel(state), instr: instructionKeyLabel(state), ask: letter, note, ret: state.platform === "macos" ? "Return" : "Enter" };
}

/** What to try, how (as nodes), what Splashdown reports when it worked,
 *  and what the box holds to start with. */
const TRIES = [
  {
    title: "Dictate",
    how: (k) => ["Click in the box, hold ", kbd(k.dict), ", say ", said("Hello, this is my first dictation"), ", and let go."],
    did: ["dictated", "instructed", "rewrote"],
  },
  {
    title: "In any app",
    how: (k) => ["Now open any app you type in (Notes, Mail, a chat), click where you'd type, hold ", kbd(k.dict), ", say something, and let go. Then come back here."],
    did: ["elsewhere"],
    noBox: true,
  },
  {
    title: "Change selected text",
    how: (k) => ["Select the sentence in the box, then hold ", kbd(k.dict), " and say ", said("make it more excited"), "."],
    did: ["rewrote"],
    start: "Our team meeting moved to Thursday afternoon.",
    ai: true,
  },
  {
    title: "Hear it read back",
    how: (k) => [
      "Select the message in the box, hold ",
      kbd(k.dict),
      " and say ",
      said("read this to me"),
      ", or ",
      said("read me a summary of this"),
      ". It's read in a natural voice, on this computer; ",
      kbd("Esc"),
      " stops it.",
    ],
    did: ["read"],
    start: "Hi Sam, the launch moved to Thursday, March 12th at 2:30 PM. Can you tell the team and book the big meeting room?",
    ai: true,
    voice: true,
  },
  {
    title: "Do things",
    how: (k) =>
      actionsOn()
        ? ["Hold ", kbd(k.dict), ", tap ", kbd(k.instr), " first, and say ", said("open Calculator"), "."]
        : ["Splashdown can also open apps, set reminders, play music and more, when you say so."],
    did: ["acted"],
    actions: true,
    noBox: true,
    ai: true,
  },
];

/** The tries for this setup: those that need the AI only when it's part of
 *  it. */
function tries() {
  return TRIES.filter((t) => !t.ai || aiPlanned());
}

function startPractice() {
  showTry();
}

/** The strip of steps, the current one's instructions, and the box. */
function showTry() {
  tryTicked = false;
  trySince = Date.now();
  const list = $("try-list");
  list.replaceChildren(
    ...tries().map((t, i) => {
      const item = document.createElement("li");
      const mark = { done: "✓", skipped: "–" }[tryState[i]] ?? String(i + 1);
      item.className = tryState[i] ?? (i === tryIndex ? "current" : "");
      item.textContent = `${mark} ${t.title}`;
      return item;
    }),
  );
  const finished = tryIndex >= tries().length;
  $("try-card").hidden = finished;
  $("try-finished").hidden = !finished;
  if (finished) {
    const tip = $("try-finished");
    tip.replaceChildren();
    const b = document.createElement("b");
    b.textContent = "That's it. ";
    tip.append(b, "To see everything else you can say, hold ", kbd(keys().dict), ", tap ", kbd(keys().instr), " and say ", said("what can I say?"), ".");
  }
  $("practice").hidden = finished || !!tries()[tryIndex]?.noBox;
  $("practice-status").hidden = finished;
  $("try-skip").hidden = finished;
  if (finished) {
    $("actions-try").hidden = true;
    updateFooter();
    return;
  }
  const t = tries()[tryIndex];
  $("try-count").textContent = `${tryIndex + 1} of ${tries().length}`;
  $("try-title").textContent = t.title;
  // The AI (or the natural voice) is still on its way: say so, and let
  // it be skipped.
  const early = (t.ai && !aiReady()) || (t.voice && !voiceHere);
  const waitingFor =
    t.ai && !aiReady()
      ? "This one needs the AI, which is still downloading (see above). It starts by itself when the AI is here."
      : "The natural voice is still downloading (68 MB). This starts by itself when it's here.";
  $("try-how").replaceChildren(...(early ? [waitingFor] : t.how(keys())));
  if (early) {
    $("actions-try").hidden = true;
    $("practice").hidden = true;
    $("practice-status").hidden = true;
    $("try-skip").hidden = false;
    updateFooter();
    return;
  }
  // Asked first; once they're on, tried.
  $("actions-try").hidden = !t.actions || actionsOn();
  $("practice-status").hidden = !!t.actions && !actionsOn();
  // While asking, Not now is the way past.
  $("try-skip").hidden = !!t.actions && !actionsOn();
  const box = $("practice");
  // Each step starts afresh: empty, or with the text it works on.
  box.value = t.start ?? "";
  const st = $("practice-status");
  st.classList.remove("ok");
  st.dataset.ready = "";
  renderPractice();
  updateFooter();
}

/** Dictation can only be tried once the speech model is here; the AI's
 *  tries once the AI is too. */
function renderPractice() {
  if (tryIndex >= tries().length || tryTicked) return;
  const t = tries()[tryIndex];
  const usable = aiReady() && (!t.voice || voiceHere);
  if ((t.ai || t.voice) && $("try-how").dataset.ai !== String(usable)) {
    $("try-how").dataset.ai = String(usable);
    if (usable) return showTry();
  }
  const box = $("practice");
  const st = $("practice-status");
  const ready = speechReady();
  if (st.dataset.ready === String(ready)) return;
  box.disabled = !ready;
  st.dataset.ready = String(ready);
  st.textContent = ready
    ? "Waiting for you to try it…"
    : "Speech to text is still downloading. This box switches on by itself.";
  if (ready) {
    trySince = Date.now();
    box.focus();
    // At the end, so "that last sentence" is the one before the cursor.
    box.setSelectionRange(box.value.length, box.value.length);
  }
}

/** Moves on: `how` is "done" or "skipped". */
function nextTry(how) {
  tryState[tryIndex] = how;
  tryIndex += 1;
  showTry();
}

$("try-skip").addEventListener("click", () => nextTry("skipped"));

async function checkTry() {
  if (step !== 4 || tryIndex >= tries().length || tryTicked || !speechReady()) return;
  const t = tries()[tryIndex];
  if ((t.ai && !aiReady()) || (t.voice && !voiceHere)) return;
  const done = await invoke("did_since", { since: trySince }).catch(() => []);
  const hit = done.find((d) => t.did.includes(d.what));
  if (!hit) return;
  tryTicked = true;
  const st = $("practice-status");
  st.textContent = t.actions ? `✓ It works — ${hit.text}` : "✓ It works";
  st.classList.add("ok");
  const index = tryIndex;
  setTimeout(() => {
    if (tryIndex === index) nextTry("done");
  }, 1500);
}

// ---------------------------------------------------------------- voice actions

function actionsOn() {
  return !!state.settings.actions?.enabled;
}

/** Yes turns voice actions on; Not now leaves them off. Either way, the
 *  question isn't asked again in this guide. */
async function answerActions(yes) {
  actionsAnswer = yes ? "yes" : "no";
  if (yes) {
    const settings = await freshSettings();
    settings.actions = { ...(settings.actions ?? {}), enabled: true };
    try {
      state = (await invoke("save_settings", { settings })).state;
    } catch (e) {
      actionsAnswer = null;
      toast(String(e), true);
      return;
    }
  }
  if (step === 4) {
    if (yes) showTry();
    else nextTry("skipped");
  }
  if (step === LAST) renderDone();
}

document.querySelectorAll("[data-actions]").forEach((b) =>
  b.addEventListener("click", () => answerActions(b.dataset.actions === "yes")),
);

// ---------------------------------------------------------------- step 5: done

/** The pill on screen was shown at the end of this guide (once: turning it
 *  off there keeps it off). */
let miniBarShown = false;

/** At the end of a first setup the pill is shown, so it's seen at least
 *  once; the switch beside it turns it off. Setup run again later leaves
 *  it as it is. */
async function showMiniBar() {
  const box = $("mini-bar");
  const on = () => state.settings.general.mini_bar === true;
  if (!miniBarShown && !state.settings.general.onboarded && !on()) {
    miniBarShown = true;
    await changeSettings((s) => { s.general.mini_bar = true; });
  }
  miniBarShown = true;
  box.checked = on();
  $("mini-bar-what").textContent = on()
    ? "Click it to dictate when you'd rather not hold a key, and again to finish. It's at the bottom of your screen now, and can be dragged anywhere."
    : "Click it to dictate when you'd rather not hold a key, and again to finish. Turn it on here, or later in Settings → General.";
}

$("mini-bar").addEventListener("change", async (e) => {
  const on = e.target.checked;
  await changeSettings((s) => { s.general.mini_bar = on ? true : null; });
  showMiniBar();
});

function renderDone() {
  showMiniBar();
  const k = keys();
  const home = $("done-home");
  home.replaceChildren();
  const b = document.createElement("b");
  if (state.platform === "macos") {
    b.textContent = "Splashdown is in the menu bar";
    home.append(b, ", at the top of your screen.");
  } else {
    b.textContent = "Splashdown is by the clock";
    home.append(b, ", at the bottom right of your screen (click ^ if you don't see it).");
  }
  const notes = $("done-notes");
  notes.replaceChildren();
  const n = document.createElement("b");
  n.textContent = "Notes and meetings: ";
  const where = state.platform === "macos" ? "the menu bar" : "Splashdown's icon by the clock";
  notes.append(n, "hold ", kbd(k.dict), " and press ", kbd(k.note), " to save a thought; record a meeting from ", where, ".");
  const help = $("done-help");
  help.replaceChildren();
  const h = document.createElement("b");
  h.textContent = "Everything else: ";
  help.append(h, "hold ", kbd(k.dict), ", tap ", kbd(k.instr), " and say ", said("what can I say?"), ".");
  const still = $("done-still");
  still.hidden = !setup?.working;
  still.textContent = "The AI is still downloading; it switches on by itself, even if you close this window.";
  // Never asked (that try skipped): ask here, once.
  $("done-actions").hidden = !aiPlanned() || actionsOn() || actionsAnswer === "no";
  const on = $("done-actions-on");
  // Turned on here but not tried yet: how to.
  const tried = tryState[tries().findIndex((t) => t.actions)] === "done";
  on.hidden = !(actionsOn() && actionsAnswer === "yes" && !tried);
  on.textContent = `✓ Voice actions are on. Hold ${k.dict}, tap ${k.instr} and say “open Calculator” to try one.`;
}

async function finish() {
  $("next").disabled = true;
  try {
    await invoke("finish_onboarding", { launchAtLogin: $("login").checked });
  } catch (e) {
    toast(String(e), true);
    $("next").disabled = false;
  }
}

// ---------------------------------------------------------------- startup

async function poll() {
  try {
    // All of it, not only the permissions: setup in the background saves
    // the providers it chose, and a later save from here mustn't put the
    // old ones back.
    state = await invoke("get_state");
    renderPermissions();
    if (step === 4 && !voiceHere) {
      voiceHere = (await invoke("reading_natural")).downloaded;
      if (voiceHere) renderPractice();
    }
  } catch {
    /* window closing */
  }
  fetchVoice();
  checkTry();
}

(async () => {
  try {
    state = await invoke("get_state");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  showDictationKey(state);
  // The demo with the keys of this computer.
  if (state.platform !== "macos") {
    const demo = document.querySelector(".demo");
    demo.src = "img/demo-win.svg";
    demo.alt = demo.alt.replace("the dictation key", "Right Ctrl");
  }
  // Opened again while getting ready: carry on showing it.
  setup = await invoke("local_setup_status").catch(() => null);
  plan = await invoke("local_setup_plan", { speechOnly: false }).catch(() => null);
  describePlan();

  // A key that's already set up (e.g. from .env.local) is offered as-is.
  const existing = existingKey();
  if (existing) choice = existing.name;
  renderChoice();
  renderPermissions();
  go(1);
  setInterval(poll, 1500);
})();

// ---------------------------------------------------------------- preview

/** Sample data for the file:// preview. Only called when window.__TAURI__ is
 *  absent (see invoke in common.js). Setting up takes a few seconds. */
async function mockInvoke(cmd, args) {
  await new Promise((r) => setTimeout(r, 200));
  mockInvoke.s ??= {
    settings: { speech: { provider: "auto", model: null }, ai: { provider: "auto", model: null, base_url: null }, general: {}, hotkeys: { dictation: null } },
    keys: [{ name: "openai", source: null, hint: null }, { name: "openrouter", source: null, hint: null }, { name: "anthropic", source: null, hint: null }, { name: "custom", source: null, hint: null }],
    permissions: { accessibility: false, microphone: "not-asked" },
    defaults: { local_speech: "nemotron" },
    key_options: [{ value: "right-alt", label: "Right Option" }, { value: "right-shift", label: "Right Shift" }],
    platform: "macos",
  };
  if (cmd === "get_state") return mockInvoke.s;
  if (cmd === "did_since") return mockInvoke.did ? [{ what: mockInvoke.did, at: Date.now(), text: "Opened Calculator" }] : [];
  if (cmd === "save_settings") {
    mockInvoke.s.settings = args.settings;
    return { state: mockInvoke.s, engine_error: null };
  }
  if (cmd === "local_setup_status") return null;
  if (cmd === "reading_natural") return { downloaded: !!mockInvoke.voice, mb: 68 };
  if (cmd === "download_model" && args.id === "phonon-en") {
    await new Promise((r) => setTimeout(r, 1500));
    mockInvoke.voice = true;
    return { state: mockInvoke.s, engine_error: null };
  }
  if (cmd === "local_setup_plan") return { cleanup: "local", small: false, download_mb: 2995, speech_mb: 495, ai_mb: 2500, free_mb: 80_000, too_little_space: false };
  if (cmd === "start_local_setup") {
    const total = 2_995_000_000;
    const status = { working: true, speech_ready: false, cleanup_ready: false, done: 0, total, speech_total: 495_000_000, error: null };
    (async () => {
      for (let i = 1; i <= 20; i++) {
        await new Promise((r) => setTimeout(r, 400));
        status.done = (total / 20) * i;
        status.speech_ready = status.done > 495_000_000;
        if (i === 20) Object.assign(status, { checking: true });
        mockEmit("setup-progress", { ...status });
      }
      await new Promise((r) => setTimeout(r, 2500));
      const ms = mockInvoke.speedMs ?? 1200;
      mockInvoke.s.settings.ai = { ...mockInvoke.s.settings.ai, provider: "local" };
      Object.assign(status, { working: false, checking: false, cleanup_ready: true, local_ms: ms, speed: ms < 2000 ? "fast" : ms <= 5000 ? "usable" : "slow" });
      mockEmit("setup-progress", { ...status });
    })();
    return status;
  }
  return { state: mockInvoke.s, engine_error: null };
}
