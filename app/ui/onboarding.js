"use strict";

// First-run setup guide (needs common.js). Opened as a plain file in a
// browser it uses the sample data in mockInvoke, for design work.
//
// Written for people who've never heard of an AI model: one obvious choice
// (set up on this computer), which starts the downloads at once and keeps
// going in the background (the `setup` module) while the rest of the guide
// carries on. API keys are there for those who have one.

const LAST = 5;
let step = 1;
let state = null;
let choice = "local"; // "local" | "openrouter" | "openai" | "anthropic" | "existing"
let plan = null; // what setting up on this computer would download
let setup = null; // how it's going, once started
let tryIndex = 0; // which of TRIES is being tried (TRIES.length: all done)
let trySince = 0; // when it began, so only what's done after counts
let tryState = []; // "done" or "skipped", by index
let tryTicked = false; // the current one worked; moving on in a moment
let actionsAnswer = null; // "yes" or "no" once asked about voice actions
let slowAnswered = false; // chose what to do about a slow built-in model
let wasWaiting = false; // step 4 was waiting for the downloads

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
    note.textContent = waiting() ? "Splashdown needs to finish getting ready first" : "";
    return;
  }
  // While trying things, moving on is a quiet choice, not the main one.
  const skipping = step === 4 && tryIndex < TRIES.length;
  next.classList.toggle("primary", !skipping);
  if (skipping) next.textContent = "Skip the rest";
}

$("back").onclick = () => go(step - 1);
$("next").onclick = async () => {
  if (step === 2 && !(await applyChoice())) return;
  if (step === LAST) return finish();
  go(step + 1);
};

// ---------------------------------------------------------------- step 2: getting ready

const KEY_CHOICES = {
  openrouter: { label: "OpenRouter", site: "openrouter.ai/keys" },
  openai: { label: "OpenAI", site: "platform.openai.com/api-keys" },
  anthropic: { label: "Anthropic", site: "platform.claude.com/settings/keys" },
};

function existingKey() {
  return state.keys.find((k) => k.source && k.name !== "custom");
}

function choiceReady() {
  if (choice === "local" || choice === "existing") return true;
  if (KEY_CHOICES[choice]) return state.keys.some((k) => k.name === choice && k.source);
  return false;
}

function gb(bytes) {
  return (bytes / 1e9).toFixed(1);
}

/** What "Set up on this computer" will do, in plain words. */
function describePlan() {
  const blurb = $("local-blurb");
  const warning = $("space-warning");
  warning.hidden = true;
  if (!plan) return;
  const size = `about ${gb(plan.download_mb * 1e6)} GB`;
  if (plan.download_mb === 0) {
    blurb.textContent = "Everything Splashdown needs is already on this computer.";
  } else if (plan.cleanup === "apple") {
    blurb.textContent = `Splashdown downloads what it needs to understand you (${size}), and tidies up your words with the Apple Intelligence built into your Mac. It takes a minute or two, and you can carry on while it does.`;
  } else {
    blurb.textContent = `Splashdown downloads what it needs to understand you and tidy up your words (${size}). It takes a few minutes, and you can carry on while it does.`;
  }
  if (plan.too_little_space) {
    warning.textContent = `This computer has ${gb(plan.free_mb * 1e6)} GB of free space, and Splashdown needs about ${gb(plan.download_mb * 1e6 + 1e9)} GB. Free up some space first (empty the ${state.platform === "macos" ? "Trash" : "Recycle Bin"}, or delete large files you don't need), or use an API key below.`;
    warning.hidden = false;
  }
}

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
  document.querySelectorAll(".card").forEach((c) => {
    c.classList.toggle("sel", c.dataset.choice === choice);
    c.setAttribute("aria-pressed", String(c.dataset.choice === choice));
  });
  const detail = $("choice-detail");
  detail.replaceChildren();
  if (KEY_CHOICES[choice]) detail.append(keyRow(choice));
  updateFooter();
}

function showKeys(open) {
  $("ai-cards").hidden = !open;
  $("show-keys").setAttribute("aria-expanded", String(open));
}

$("show-keys").onclick = () => showKeys($("ai-cards").hidden);

function pick(card) {
  choice = card.dataset.choice;
  renderChoice();
}

document.querySelectorAll(".card").forEach((c) => c.addEventListener("click", () => pick(c)));

/** Saves the chosen setup and starts any downloads. Returns false to stay
 *  on the step. */
async function applyChoice() {
  $("next").disabled = true;
  const fail = () => {
    $("next").disabled = false;
    return false;
  };
  try {
    if (choice === "local") {
      // Saved by the setup itself, as each part arrives.
      setup = await invoke("start_local_setup", { speechOnly: false });
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
  return true;
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
  return ` · about ${minutes} minute${minutes === 1 ? "" : "s"} left`;
}

/** Can dictation be tried yet? */
function speechReady() {
  return !setup || setup.speech_ready;
}

/** The strip at the top saying how getting ready is going, on step 3 (step
 *  4 has its own, bigger one). */
function renderSetup() {
  const strip = $("getting-ready");
  strip.hidden = !setup || step !== 3;
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
  if (setup.error) {
    text.textContent = setup.error;
  } else if (!setup.working) {
    text.textContent = "✓ Splashdown is ready.";
  } else if (!setup.speech_ready) {
    text.textContent = `Getting Splashdown ready… ${amount}${timeLeft()}`;
  } else {
    text.textContent = `✓ You can try it now. Still downloading what tidies up your words: ${amount}${timeLeft()}`;
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

// ---------------------------------------------------------------- step 4: getting ready

/** Downloads or the speed check still going (or stopped by an error):
 *  nothing can be tried yet, and the guide waits here. */
function waiting() {
  return !!setup && (setup.working || setup.checking || !!setup.error);
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
  $("try-h1").textContent = wait ? "Almost there" : slow ? "One thing first" : "Try it";
  if (wait) {
    wasWaiting = true;
    const pct = setup.total ? Math.min(1, setup.done / setup.total) : 0;
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
      $("wait-sub").textContent = "Splashdown is tidying up a test dictation. This takes a few seconds.";
      $("wait-amount").textContent = "";
    } else {
      $("wait-title").textContent = "Getting Splashdown ready";
      $("wait-sub").textContent = setup.speech_ready
        ? "Now downloading what tidies up your words. You can try everything as soon as it's done."
        : "Downloading what Splashdown needs to understand you and tidy up your words.";
      $("wait-amount").textContent = setup.total
        ? `${Math.floor(pct * 100)}% · ${gb(setup.done)} of ${gb(setup.total)} GB${timeLeft()}`
        : "Starting…";
    }
  } else if (slow) {
    $("slow-text").textContent = `Each dictation would take about ${secondsOf(setup.local_ms)} seconds to be tidied up here, because the built-in model needs a fast processor or graphics chip. A cloud service is much quicker.`;
  } else {
    const note = $("speed-note");
    note.hidden = setup?.speed !== "usable" || state.settings.ai?.provider !== "local";
    if (!note.hidden) {
      note.textContent = `Tidying up takes about ${secondsOf(setup.local_ms)} seconds on this computer. A cloud key makes it quicker: Settings → Providers.`;
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

/** Saves a change to the settings; true if it worked. */
async function changeSettings(change) {
  const settings = structuredClone(state.settings);
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
  // Voice still understood here; only the tidying up moves.
  await slowChosen((s) => {
    s.ai = { ...s.ai, provider: "openrouter", model: null };
  });
  toast("Key verified · tidying up now uses OpenRouter");
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
  return { dict: dictationKeyLabel(state), instr: "Right Shift", ask: letter, ret: state.platform === "macos" ? "Return" : "Enter" };
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
    title: "Change selected text",
    how: (k) => ["Select the sentence in the box, then hold ", kbd(k.dict), " and say ", said("make it more excited"), "."],
    did: ["rewrote"],
    start: "Our team meeting moved to Thursday afternoon.",
  },
  {
    title: "Add an instruction",
    how: (k) => ["Hold ", kbd(k.dict), " and say ", said("milk, eggs and bread"), ", then tap ", kbd(k.instr), ", say ", said("as a bulleted list"), " and let go."],
    did: ["instructed"],
  },
  {
    title: "Press Return when it's typed",
    how: (k) => ["Hold ", kbd(k.dict), ", say something, and tap ", kbd(k.ret), " before you let go. In a chat app, that sends it."],
    did: ["sent"],
  },
  {
    title: "Fix it by voice",
    how: (k) => ["Hold ", kbd(k.dict), ", tap ", kbd(k.instr), " first, then say ", said("delete that last sentence"), "."],
    did: ["edited"],
    start: "This sentence stays. This one goes.",
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
  },
  {
    title: "Ask a question",
    how: (k) => ["Hold ", kbd(k.dict), ", press ", kbd(k.ask), ", and ask ", said("what's the German word for deadline?"), ". The answer shows in a panel; Esc closes it."],
    did: ["asked"],
    noBox: true,
  },
];

function startPractice() {
  showTry();
}

/** The strip of steps, the current one's instructions, and the box. */
function showTry() {
  tryTicked = false;
  trySince = Date.now();
  const list = $("try-list");
  list.replaceChildren(
    ...TRIES.map((t, i) => {
      const item = document.createElement("li");
      const mark = { done: "✓", skipped: "–" }[tryState[i]] ?? String(i + 1);
      item.className = tryState[i] ?? (i === tryIndex ? "current" : "");
      item.textContent = `${mark} ${t.title}`;
      return item;
    }),
  );
  const finished = tryIndex >= TRIES.length;
  $("try-card").hidden = finished;
  $("try-finished").hidden = !finished;
  $("practice").hidden = finished || !!TRIES[tryIndex]?.noBox;
  $("practice-status").hidden = finished;
  $("try-skip").hidden = finished;
  if (finished) {
    $("actions-try").hidden = true;
    updateFooter();
    return;
  }
  const t = TRIES[tryIndex];
  $("try-count").textContent = `${tryIndex + 1} of ${TRIES.length}`;
  $("try-title").textContent = t.title;
  $("try-how").replaceChildren(...t.how(keys()));
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

/** Dictation can only be tried once the speech model is here. */
function renderPractice() {
  if (tryIndex >= TRIES.length || tryTicked) return;
  const box = $("practice");
  const st = $("practice-status");
  const ready = speechReady();
  if (st.dataset.ready === String(ready)) return;
  box.disabled = !ready;
  st.dataset.ready = String(ready);
  st.textContent = ready
    ? "Waiting for you to try it…"
    : "Almost ready: Splashdown is still downloading what it needs to understand you. This box switches on by itself.";
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
  if (step !== 4 || tryIndex >= TRIES.length || tryTicked || !speechReady()) return;
  const t = TRIES[tryIndex];
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
    const settings = structuredClone(state.settings);
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

function renderDone() {
  const home = state.platform === "macos"
    ? "Splashdown lives in the menu bar, at the top of the screen."
    : "Splashdown lives by the clock, at the bottom right of the screen (click ^ if you don't see it).";
  const still = setup?.working
    ? " It's still finishing a download: tidying up your words switches on by itself when it's done, even if you close this window."
    : "";
  $("done-sub").textContent = `${home}${still} A few things worth knowing:`;
  // Never asked (step 6 skipped): ask here, once.
  $("done-actions").hidden = actionsOn() || actionsAnswer === "no";
  const on = $("done-actions-on");
  // Turned on here but not tried yet: how to.
  const tried = tryState[TRIES.findIndex((t) => t.actions)] === "done";
  on.hidden = !(actionsOn() && actionsAnswer === "yes" && !tried);
  on.textContent = `✓ Voice actions are on. Hold ${dictationKeyLabel(state)}, tap Right Shift and say “open Calculator” to try one.`;
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
    const fresh = await invoke("get_state");
    state.permissions = fresh.permissions;
    renderPermissions();
  } catch {
    /* window closing */
  }
  checkTry();
}

(async () => {
  state = await invoke("get_state");
  showDictationKey(state);
  // Opened again while getting ready: carry on showing it.
  setup = await invoke("local_setup_status").catch(() => null);
  plan = await invoke("local_setup_plan", { speechOnly: false }).catch(() => null);
  describePlan();

  // A key that's already set up (e.g. from .env.local) is offered as-is.
  const existing = existingKey();
  if (existing) {
    const name = KEY_CHOICES[existing.name]?.label ?? existing.name;
    const card = document.createElement("button");
    card.className = "card";
    card.dataset.choice = "existing";
    card.innerHTML = `<span class="ic" aria-hidden="true">✓</span><b>Use my ${name} key</b><p>Already set up (…${existing.hint}).</p><span class="badge ok">Ready</span>`;
    card.addEventListener("click", () => pick(card));
    $("ai-cards").prepend(card);
    showKeys(true);
    choice = "existing";
  }
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
    key_options: [{ value: "right-alt", label: "Right Option" }],
    platform: "macos",
  };
  if (cmd === "get_state") return mockInvoke.s;
  if (cmd === "did_since") return mockInvoke.did ? [{ what: mockInvoke.did, at: Date.now(), text: "Opened Calculator" }] : [];
  if (cmd === "save_settings") {
    mockInvoke.s.settings = args.settings;
    return { state: mockInvoke.s, engine_error: null };
  }
  if (cmd === "local_setup_status") return null;
  if (cmd === "local_setup_plan") return { cleanup: "local", small: false, download_mb: 2995, free_mb: 80_000, too_little_space: false };
  if (cmd === "start_local_setup") {
    const total = 2_995_000_000;
    const status = { working: true, speech_ready: false, cleanup_ready: false, done: 0, total, error: null };
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
