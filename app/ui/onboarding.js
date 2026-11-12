"use strict";

// --------------------------------------------------------------- pictures

// Each setup picture comes in two files, for a light page and (-dark) for a
// dark one. A picture shown with <img> isn't told the page is dark in this
// app's browser engine on a Mac, so a rule inside the picture can't choose.
const darkPage = window.matchMedia("(prefers-color-scheme: dark)");

/** Shows the setup picture `name` in `img`, in the colours that suit the page. */
function showPicture(img, name) {
  img.dataset.pic = name;
  img.src = `img/${name}${darkPage.matches ? "-dark" : ""}.svg`;
}

function showPictures() {
  document.querySelectorAll("img[data-pic]").forEach((img) => showPicture(img, img.dataset.pic));
}
showPictures();
// The system's appearance can change while the guide is open.
darkPage.addEventListener("change", showPictures);

// First-run setup guide (needs common.js). Opened as a plain file in a
// browser it uses the sample data in mockInvoke, for design work.
//
// Written for people who've never heard of an AI model, and so that
// nobody can get it wrong: one thing per screen, and no decisions on the
// way. Everything runs on this computer, so the first button starts the
// downloads (speech to text first, then what tidies up the words, then the
// reading voice), and the rules about disk space, memory and speed are
// settled by Splashdown (setup.rs), not asked. Each screen moves on by
// itself once its one thing is done. Closing the guide part-way keeps the
// place: it opens there again.

/** The screens, in order. Some are skipped when there's nothing to do on
 *  them (see `skipped`); "key" (behind "I have an API key") and "tour"
 *  (the optional extras, after finishing) are off the main path. */
const MAIN = ["welcome", "microphone", "accessibility", "ready", "hello", "home"];

let screen = null;
let state = null;
let choice = "local"; // "local" | "openrouter" | "openai" | "anthropic"
let plan = null; // what setting up on this computer would download
let setup = null; // how it's going, once started

const mac = () => state?.platform === "macos";

// ---------------------------------------------------------------- navigation

/** Screens with nothing to do right now. */
function skipped(name) {
  const mic = state?.permissions?.microphone;
  switch (name) {
    // Windows asks for nothing, unless the microphone is turned off for
    // desktop apps (then dictation would wait forever).
    case "microphone":
      return mac() ? mic === "granted" : mic !== "denied";
    case "accessibility":
      return !mac() || !!state?.permissions?.accessibility;
    case "ready":
      return !waiting();
    default:
      return false;
  }
}

/** Shows screen `name`, or the nearest one after it (before it, going
 *  `back`) that has something to do. */
function go(name, back = false) {
  let i = MAIN.indexOf(name);
  if (i > 0) {
    while (i > 0 && skipped(MAIN[i])) i += back ? -1 : 1;
    name = MAIN[i];
  }
  // Going back to the start from a key: the key screen is the start.
  if (back && name === "welcome" && choice !== "local") name = "key";
  if (screen && screen !== name) leave(screen);
  screen = name;
  document.querySelectorAll(".ob-step").forEach((s) => (s.hidden = s.dataset.screen !== name));
  $("back").hidden = name === "welcome" || name === "tour";
  renderSteps();
  remember(name);
  enter(name);
  renderNote();
}

function next() {
  const i = MAIN.indexOf(screen);
  go(MAIN[Math.min(MAIN.length - 1, i + 1)]);
}

$("back").onclick = () => {
  if (screen === "key") return go("welcome");
  go(MAIN[Math.max(0, MAIN.indexOf(screen) - 1)], true);
};

/** The dots at the top: one per screen on the way, lit up to this one. */
function renderSteps() {
  const box = $("steps");
  box.hidden = screen === "tour";
  const shown = MAIN.filter((n) => n === screen || !skipped(n));
  const at = Math.max(0, shown.indexOf(screen === "key" ? "welcome" : screen));
  box.replaceChildren(
    ...shown.map((_, i) => {
      const dot = document.createElement("i");
      dot.className = i <= at ? "on" : "";
      return dot;
    }),
  );
}

/** Kept in the settings, so the guide opens here again if it's closed. */
function remember(name) {
  if (!MAIN.includes(name) && name !== "key") return;
  if (state?.settings?.general?.onboarded) return;
  invoke("set_onboarding_step", { step: name, choice }).catch(() => {});
}

function enter(name) {
  ({
    welcome: renderWelcome,
    key: renderChoice,
    microphone: renderMic,
    accessibility: renderAx,
    ready: renderReady,
    hello: startHello,
    home: enterHome,
    tour: startTour,
  })[name]?.();
}

function leave(name) {
  if (name === "hello") stopHello();
  if (name === "home") invoke("point_at_tray", { on: false }).catch(() => {});
}

// ---------------------------------------------------------------- sizes and text

function gb(bytes) {
  return (bytes / 1e9).toFixed(1);
}

/** "495 MB", "3.1 GB". */
function size(mb) {
  return mb < 1000 ? `${Math.round(mb)} MB` : `${(mb / 1000).toFixed(1)} GB`;
}

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

function bin() {
  return mac() ? "Trash" : "Recycle Bin";
}

// ---------------------------------------------------------------- 1 · welcome

/** Says what happens next, honestly: how much it downloads. With no room
 *  even for speech to text, how much to free instead. */
function renderWelcome() {
  const nextUp = $("welcome-next");
  const box = $("space-box");
  const start = $("welcome-go");
  box.hidden = true;
  nextUp.hidden = false;
  start.disabled = false;
  if (!plan) {
    nextUp.textContent = "First it downloads what it needs, then it works without the internet.";
    return;
  }
  if (plan.room === "none") {
    nextUp.hidden = true;
    box.hidden = false;
    start.disabled = true;
    $("space-text").textContent = `There isn't enough free space on this computer. Free up about ${size(Math.max(100, plan.free_up_mb))} (empty the ${bin()}, or delete large files you don't need), then check again.`;
    return;
  }
  const mb = plan.room === "speech" ? plan.speech_mb : plan.download_mb;
  nextUp.textContent =
    mb === 0
      ? "Everything it needs is already here, so it works without the internet."
      : `First it downloads what it needs (${size(mb)}), then it works without the internet.`;
}

$("welcome-go").onclick = async () => {
  const btn = $("welcome-go");
  btn.disabled = true;
  try {
    // Saved by the setup itself, as each part arrives.
    setup = await invoke("start_local_setup", { speechOnly: false });
    choice = "local";
    samples = [];
  } catch (e) {
    // Space ran out since the page opened: say so where the button is.
    $("welcome-next").hidden = true;
    $("space-box").hidden = false;
    $("space-text").textContent = String(e);
    btn.disabled = true;
    return;
  }
  next();
};

$("space-again").onclick = async () => {
  plan = await invoke("local_setup_plan", { speechOnly: false }).catch(() => plan);
  renderWelcome();
  if (plan?.room !== "none") toast("There's room now");
};

$("welcome-key").onclick = () => {
  if (choice === "local") choice = existingKey()?.name ?? "openrouter";
  go("key");
};

// ---------------------------------------------------------------- behind "I have an API key"

const KEY_CHOICES = {
  openrouter: { label: "OpenRouter", site: "openrouter.ai/keys" },
  openai: { label: "OpenAI", site: "platform.openai.com/api-keys" },
  anthropic: { label: "Anthropic", site: "platform.claude.com/settings/keys" },
};

/** What each key does, in one line. */
const KEY_NOTES = {
  openrouter: "One key for many models: the best cleanup and formatting, a few cents a day even with heavy use.",
  openai: "Speech to text and cleanup with your OpenAI account.",
  anthropic: "Claude cleans up your words; speech to text runs on this computer (a 0.5 GB download).",
};

function existingKey() {
  return state.keys.find((k) => k.source && KEY_CHOICES[k.name]);
}

function keySaved() {
  return state.keys.some((k) => k.name === choice && k.source);
}

function keyRow(name) {
  const info = KEY_CHOICES[name];
  const key = state.keys.find((k) => k.name === name);
  const row = document.createElement("div");
  row.className = "row";
  const where = mac() ? "It's kept in your Keychain." : "It's kept safely on this computer.";
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
  };
  save.onclick = submit;
  input.onkeydown = (e) => e.key === "Enter" && submit();
  row.append(input, save);
  setTimeout(() => input.focus(), 0);
  return row;
}

function renderChoice() {
  if (!KEY_CHOICES[choice]) choice = existingKey()?.name ?? "openrouter";
  document.querySelectorAll("#key-provider button").forEach((b) => {
    b.setAttribute("aria-checked", String(b.dataset.provider === choice));
    b.classList.toggle("on", b.dataset.provider === choice);
  });
  $("choice-detail").replaceChildren(keyRow(choice));
  $("key-note").textContent = KEY_NOTES[choice] ?? "";
  $("key-go").disabled = !keySaved();
  $("key-wait").hidden = keySaved();
}

document.querySelectorAll("#key-provider button").forEach((b) =>
  b.addEventListener("click", () => {
    choice = b.dataset.provider;
    renderChoice();
  }),
);

$("key-go").onclick = async () => {
  const btn = $("key-go");
  btn.disabled = true;
  try {
    if (choice === "anthropic") {
      // Claude only tidies up; the voice is understood on this computer.
      await saveProviders("auto", "anthropic");
      setup = await invoke("start_local_setup", { speechOnly: true });
      samples = [];
    } else {
      await saveProviders("auto", "auto");
    }
  } catch (e) {
    toast(String(e), true);
    btn.disabled = false;
    return;
  }
  go("microphone");
};

async function saveProviders(speech, ai) {
  const fresh = await invoke("get_state");
  const settings = structuredClone(fresh.settings);
  settings.speech = { provider: speech, model: null };
  settings.ai = { ...settings.ai, provider: ai, model: null };
  state = (await invoke("save_settings", { settings })).state;
}

// ---------------------------------------------------------------- 2 · microphone

let advancing = false;

/** Moves on a moment after `screenName` is done, so the tick is seen. */
function advanceFrom(screenName, ms = 700) {
  if (advancing) return;
  advancing = true;
  setTimeout(() => {
    advancing = false;
    if (screen === screenName) next();
  }, ms);
}

function renderMic() {
  const mic = state.permissions.microphone;
  const windows = !mac();
  const granted = windows ? mic !== "denied" : mic === "granted";
  $("mic-go").hidden = windows || mic !== "not-asked";
  $("mic-ok").hidden = !granted;
  $("mic-denied").hidden = granted || (!windows && mic === "not-asked");
  if (windows) {
    $("mic-why").textContent = "Microphone access is off for desktop apps on this computer, so Splashdown can't hear you yet.";
    $("mic-denied-text").replaceChildren(
      "In Settings → Privacy & security → Microphone, turn on ",
      Object.assign(document.createElement("b"), { textContent: "Let desktop apps access your microphone" }),
      ". This page moves on by itself.",
    );
    $("mic-open").textContent = "Open Settings";
  } else {
    $("mic-denied-text").replaceChildren(
      "The microphone is switched off for Splashdown. In System Settings → Privacy & Security → Microphone, switch on ",
      Object.assign(document.createElement("b"), { textContent: "Splashdown" }),
      ". This page moves on by itself.",
    );
  }
  if (granted) advanceFrom("microphone");
}

$("mic-go").onclick = async () => {
  const btn = $("mic-go");
  btn.disabled = true;
  try {
    await invoke("request_microphone");
  } catch (e) {
    toast(String(e), true);
  }
  btn.disabled = false;
  await refresh();
};

$("mic-open").onclick = () => call("open_permission", { permission: "microphone" });

// ---------------------------------------------------------------- 3 · accessibility

/** When System Settings was opened, for the help that follows if the
 *  switch isn't on 20 seconds later. */
let axAskedAt = 0;

function renderAx() {
  const on = !!state.permissions.accessibility;
  $("ax-ok").hidden = !on;
  $("ax-go").hidden = on;
  $("ax-wait").hidden = on || !axAskedAt;
  $("ax-help").hidden = on || !axAskedAt || Date.now() - axAskedAt < 20_000;
  if (on && screen === "accessibility" && !advancing) {
    // Back to the front, from System Settings.
    invoke("open_onboarding").catch(() => {});
    advanceFrom("accessibility", 900);
  }
}

$("ax-go").onclick = async () => {
  // The system's own prompt adds Splashdown to the list (so only the
  // switch is left to turn on); the page is opened straight away too, in
  // case that prompt was shown before and doesn't come again.
  await invoke("request_accessibility").catch(() => {});
  call("open_permission", { permission: "accessibility" });
  axAskedAt = Date.now();
  renderAx();
};

// ---------------------------------------------------------------- 4 · getting ready

/** Recent progress, to say how long is left, and when bytes last came. */
let samples = [];
let movedAt = Date.now();

listen("setup-progress", (event) => {
  const before = setup?.done;
  setup = event.payload;
  if (setup.done !== before) movedAt = Date.now();
  samples.push({ t: Date.now(), done: setup.done });
  samples = samples.filter((s) => Date.now() - s.t < 10_000);
  onSetup();
});

function onSetup() {
  if (screen === "ready") renderReady();
  if (screen === "welcome" && setup?.speech_ready === false && setup?.error) renderWelcome();
  renderNote();
}

function timeLeft() {
  if (samples.length < 2) return "";
  const first = samples[0];
  const last = samples[samples.length - 1];
  const rate = (last.done - first.done) / ((last.t - first.t) / 1000);
  if (!(rate > 0)) return "";
  const end = setup.speech_ready ? setup.total : setup.speech_total || setup.total;
  const seconds = (end - setup.done) / rate;
  if (seconds < 60) return " · less than a minute left";
  return ` · about ${plural(Math.round(seconds / 60), "minute")} left`;
}

/** Speech to text isn't here yet (or its download stopped): nothing can be
 *  tried, so the guide waits on "ready". The rest follows in the
 *  background. */
function waiting() {
  return !!setup && !setup.speech_ready;
}

function renderReady() {
  if (!waiting()) {
    if (screen === "ready") advanceFrom("ready", 400);
    return;
  }
  const speechTotal = setup.speech_total || setup.total;
  const pct = speechTotal ? Math.min(1, setup.done / speechTotal) : 0;
  const stalled = setup.working && Date.now() - movedAt > 30_000;
  $("ready-progress").hidden = !!setup.error;
  $("ready-bar").style.width = `${Math.floor(pct * 100)}%`;
  $("ready-retry").hidden = !setup.error;
  if (setup.error) {
    $("ready-title").textContent = "The download stopped";
    $("ready-sub").textContent = setup.error;
    $("ready-amount").textContent = "";
  } else {
    $("ready-title").textContent = "Getting ready";
    $("ready-sub").textContent = "Downloading what Splashdown needs to understand you. This page moves on by itself.";
    $("ready-amount").textContent = stalled
      ? "Nothing has arrived for a while. Check the internet connection: the download carries on by itself when it's back."
      : speechTotal
        ? `${Math.floor(pct * 100)}% · ${gb(setup.done)} of ${gb(speechTotal)} GB${timeLeft()}`
        : "Starting…";
  }
}

async function retrySetup() {
  try {
    setup = await invoke("start_local_setup", { speechOnly: choice === "anthropic" });
    samples = [];
    movedAt = Date.now();
    onSetup();
  } catch (e) {
    toast(String(e), true);
  }
}

$("ready-retry").onclick = retrySetup;

// ---------------------------------------------------------------- one quiet sentence

/** Said once, where it fits (Say hello, Where it lives), when something
 *  was settled for them: only speech, for lack of space; tidying up off,
 *  as it's too slow here; the rest still on its way, or stopped. */
function renderNote() {
  const box = $("setup-note");
  const text = $("setup-note-text");
  const act = $("setup-note-act");
  act.hidden = true;
  box.hidden = true;
  if (!setup || !["hello", "home"].includes(screen)) return;
  const say = (words, action, onclick) => {
    box.hidden = false;
    text.textContent = words;
    if (action) {
      act.hidden = false;
      act.textContent = action;
      act.onclick = onclick;
    }
  };
  if (setup.slow_off) {
    say(
      "Tidying up your words is slow on this computer, so they're typed just as you say them. You can turn it on in Settings → AI Models.",
      "Keep it on anyway",
      keepSlow,
    );
  } else if (setup.no_room_for_ai) {
    say("There wasn't room for the part that tidies up your words, so they're typed just as you say them. Add it in Settings → AI Models once there's more space.");
  } else if (setup.error && setup.speech_ready) {
    say(`The rest of the download stopped. ${setup.error}`, "Try again", retrySetup);
  } else if (screen === "home" && setup.working && setup.speech_ready) {
    const rest = setup.total - (setup.speech_total ?? 0);
    const done = Math.max(0, setup.done - (setup.speech_total ?? 0));
    say(
      setup.checking
        ? "Checking how fast this computer tidies up your words. It takes a few seconds, even if you close this window."
        : `The part that tidies up your words is still downloading (${gb(done)} of ${gb(rest)} GB). It switches on by itself, even if you close this window.`,
    );
  }
}

async function keepSlow() {
  try {
    await invoke("keep_slow_tidy_up");
    setup = { ...setup, slow_off: false };
    toast("Tidying up is on");
  } catch (e) {
    toast(String(e), true);
  }
  renderNote();
}

// ---------------------------------------------------------------- 5 · say hello

let helloSince = 0;
let helloTimer = null;
let helloTicked = false;
let everHeld = false;
let everHeard = false;
let heardAt = 0;

function keys() {
  const letter = (state.settings.hotkeys.ask ?? "a").toUpperCase();
  // The note letter: N, or M when hands-free has N (as `Hotkeys::note_key`).
  const hands = (state.settings.hotkeys.hands_free || "h").toLowerCase();
  const note = (state.settings.hotkeys.note || (hands === "n" ? "m" : "n")).toUpperCase();
  return { dict: dictationKeyLabel(state), instr: instructionKeyLabel(state), ask: letter, note };
}

function startHello() {
  const k = keys().dict;
  $("hello-how").replaceChildren("Hold ", kbd(k), ", say ", said("Hello, this is my first dictation"), ", and let go.");
  // The picture shows the usual key; a key chosen in Settings is named
  // above instead.
  const usual = (state.settings.hotkeys.dictation ?? defaultDictationKey(state.platform)) === defaultDictationKey(state.platform);
  $("key-fig").hidden = !usual;
  const img = $("key-img");
  if (!mac()) {
    showPicture(img, "setup-key-win");
    img.alt = "A keyboard with the Right Ctrl key, at the right end of the bottom row, highlighted.";
  }
  helloSince = Date.now();
  helloTicked = false;
  everHeld = false;
  everHeard = false;
  heardAt = 0;
  const box = $("practice");
  box.value = "";
  box.disabled = false;
  box.focus();
  $("hello-status").textContent = "";
  $("hello-status").classList.remove("ok");
  $("hello-help").hidden = true;
  showSigns({ held: false, level: 0 });
  clearInterval(helloTimer);
  helloTimer = setInterval(watchHello, 150);
}

function stopHello() {
  clearInterval(helloTimer);
  helloTimer = null;
}

/** Five bars for the level: speech sits around 0.01 to 0.3 (an RMS), so
 *  it's shown on a decibel scale, from -50 dB to -10 dB. */
function bars(level) {
  if (!(level > 0)) return 0;
  const db = 20 * Math.log10(level);
  return Math.max(0, Math.min(5, Math.round(((db + 50) / 40) * 5)));
}

function showSigns({ held, level }) {
  const key = $("sign-key");
  key.classList.toggle("on", held);
  key.classList.toggle("seen", everHeld && !held);
  $("sign-key-text").textContent = held ? `${keys().dict} held` : everHeld ? `${keys().dict} noticed` : `${keys().dict} not held`;
  const lit = held ? bars(level) : 0;
  document.querySelectorAll("#sign-mic .meter i").forEach((b, i) => b.classList.toggle("on", i < lit));
  const hearing = held && Date.now() - heardAt < 500;
  $("sign-mic").classList.toggle("on", hearing);
  $("sign-mic-text").textContent = hearing ? "Hearing you" : everHeard ? "Heard you" : "Not heard yet";
}

let lastDidCheck = 0;

async function watchHello() {
  if (screen !== "hello" || helloTicked) return;
  let signs;
  try {
    signs = await invoke("try_signs");
  } catch {
    return;
  }
  if (signs.held_at >= helloSince) everHeld = true;
  if (signs.heard_at >= helloSince) {
    everHeard = true;
    heardAt = Math.max(heardAt, signs.heard_at);
  }
  showSigns(signs);
  if (Date.now() - helloSince > 15_000) explainHello();
  if (Date.now() - lastDidCheck < 500) return;
  lastDidCheck = Date.now();
  const done = await invoke("did_since", { since: helloSince }).catch(() => []);
  if (!done.some((d) => ["dictated", "instructed", "rewrote"].includes(d.what)) || helloTicked) return;
  helloTicked = true;
  stopHello();
  $("hello-help").hidden = true;
  const st = $("hello-status");
  st.textContent = "✓ It works. You just dictated.";
  st.classList.add("ok");
  advanceFrom("hello", 1800);
}

/** Nothing has worked after a while: which sign is missing, and what to
 *  do about it. */
function explainHello() {
  const k = keys().dict;
  const where = mac()
    ? "the Option key just right of the space bar"
    : "the Ctrl key at the right end of the bottom row";
  let words;
  if (!everHeld && mac() && !state.permissions.accessibility) {
    words = "Splashdown isn't allowed to notice keys yet. Go back one step and switch it on in System Settings.";
  } else if (!everHeld) {
    words = `Splashdown hasn't noticed ${k} yet. Click in the box above, then hold ${k} (${where}) the whole time you speak.`;
  } else if (!everHeard) {
    words = mac()
      ? "The key works, but the microphone hears nothing. Check it isn't muted, and that Splashdown is allowed in System Settings → Privacy & Security → Microphone."
      : "The key works, but the microphone hears nothing. Check it isn't muted, and that the right one is chosen in Splashdown's Settings → General.";
  } else {
    words = `The key and the microphone both work. Keep holding ${k} until you've finished speaking, then let go: your words appear a moment later.`;
  }
  $("hello-help-text").textContent = words;
  $("hello-help").hidden = false;
}

$("hello-skip").onclick = () => {
  stopHello();
  next();
};

// ---------------------------------------------------------------- 6 · where it lives

/** The pill is shown at the end of a first setup, so it's seen at least
 *  once (Settings turns it off). Once per guide. */
let pillShown = false;

async function enterHome() {
  const img = $("home-img");
  if (mac()) {
    $("home-title").textContent = "Splashdown lives in the menu bar";
    $("home-sub").textContent = "It has no window of its own. Click its icon at the top right of your screen for Settings, notes and more.";
    $("home-taskbar").hidden = true;
  } else {
    $("home-title").textContent = "Splashdown lives by the clock";
    $("home-sub").textContent = "It has no window of its own. Its icon is by the clock, at the bottom right of your screen. If it's hidden under ^, drag it onto the taskbar so it stays in sight.";
    showPicture(img, "setup-tray-win");
    img.alt = "The bottom right of a Windows screen: clicking ^ by the clock shows Splashdown's icon, which is dragged onto the taskbar to keep it in sight.";
    $("home-taskbar").hidden = false;
  }
  $("home-pill").textContent = "The small pill at the bottom of your screen works too: click it to start talking, and again to finish.";
  $("home-pointing").hidden = true;
  if (!pillShown && !state.settings.general.onboarded && state.settings.general.mini_bar !== true) {
    pillShown = true;
    await changeSettings((s) => {
      s.general.mini_bar = true;
    });
  }
  // An arrow at the real icon, where its place can be known.
  const pointing = await invoke("point_at_tray", { on: true }).catch(() => false);
  if (screen === "home") $("home-pointing").hidden = !pointing;
}

$("home-taskbar").onclick = () => call("open_taskbar_settings");

$("home-done").onclick = async () => {
  $("home-done").disabled = true;
  if (!(await finish(true))) $("home-done").disabled = false;
};

$("home-tour").onclick = async () => {
  if (await finish(false)) go("tour");
};

/** Marks setup as done (start at login stays on). `close`: the window too. */
async function finish(close) {
  try {
    await invoke("finish_onboarding", { launchAtLogin: true, close });
    state.settings.general.onboarded = true;
    return true;
  } catch (e) {
    toast(String(e), true);
    return false;
  }
}

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

// ---------------------------------------------------------------- the optional tour

// The other things to try, one at a time in one practice box, each ticked
// off when Splashdown says it worked (`did_since`), and each skippable.

let tryIndex = 0; // which of tries() is being tried (its length: all done)
let trySince = 0; // when it began, so only what's done after counts
let tryState = []; // "done" or "skipped", by index
let tryTicked = false; // the current one worked; moving on in a moment
let voiceHere = false; // the natural reading voice is downloaded

/** What to try, how (as nodes), what Splashdown reports when it worked,
 *  and what the box holds to start with. */
const TRIES = [
  {
    title: "In any app",
    how: (k) => ["Open any app you type in (Notes, Mail, a chat), click where you'd type, hold ", kbd(k.dict), ", say something, and let go. Then come back here."],
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

/** The tidy-up is part of this setup (not turned off). */
function aiPlanned() {
  return state.settings.ai?.provider !== "off";
}

/** It's here to use: a cloud key, or the download is done. */
function aiReady() {
  return aiPlanned() && (!setup || !!setup.cleanup_ready || (!setup.working && !setup.error));
}

/** The tries for this setup: those that need tidying up only with it. */
function tries() {
  return TRIES.filter((t) => !t.ai || aiPlanned());
}

function actionsOn() {
  return !!state.settings.actions?.enabled;
}

function startTour() {
  tryIndex = 0;
  tryState = [];
  showTry();
  // The reading voice normally came with setup; fetch it if not.
  if (aiPlanned() && !voiceHere) {
    invoke("download_model", { id: "phonon-en" })
      .then(() => {
        voiceHere = true;
        if (screen === "tour") renderPractice();
      })
      .catch(() => {});
  }
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
  $("tour-box").hidden = finished || !!tries()[tryIndex]?.noBox;
  $("practice-status").hidden = finished;
  $("try-skip").hidden = finished;
  if (finished) {
    $("actions-try").hidden = true;
    return;
  }
  const t = tries()[tryIndex];
  $("try-count").textContent = `${tryIndex + 1} of ${tries().length}`;
  $("try-title").textContent = t.title;
  // Tidying up (or the natural voice) is still on its way: say so, and
  // let it be skipped.
  const early = (t.ai && !aiReady()) || (t.voice && !voiceHere);
  const waitingFor =
    t.ai && !aiReady()
      ? "This one needs the part that tidies up your words, which is still downloading. It starts by itself when that's here."
      : "The natural voice is still downloading (68 MB). This starts by itself when it's here.";
  $("try-how").replaceChildren(...(early ? [waitingFor] : t.how(keys())));
  if (early) {
    $("actions-try").hidden = true;
    $("tour-box").hidden = true;
    $("practice-status").hidden = true;
    $("try-skip").hidden = false;
    return;
  }
  // Asked first; once they're on, tried.
  $("actions-try").hidden = !t.actions || actionsOn();
  $("practice-status").hidden = !!t.actions && !actionsOn();
  // While asking, Not now is the way past.
  $("try-skip").hidden = !!t.actions && !actionsOn();
  const box = $("tour-box");
  // Each step starts afresh: empty, or with the text it works on.
  box.value = t.start ?? "";
  const st = $("practice-status");
  st.classList.remove("ok");
  st.dataset.ready = "";
  renderPractice();
}

/** Tries that need tidying up (or the voice) start once it's here. */
function renderPractice() {
  if (tryIndex >= tries().length || tryTicked) return;
  const t = tries()[tryIndex];
  const usable = aiReady() && (!t.voice || voiceHere);
  if ((t.ai || t.voice) && $("try-how").dataset.ai !== String(usable)) {
    $("try-how").dataset.ai = String(usable);
    if (usable) return showTry();
  }
  const st = $("practice-status");
  if (st.dataset.ready === "true") return;
  st.dataset.ready = "true";
  st.textContent = "Waiting for you to try it…";
  trySince = Date.now();
  const box = $("tour-box");
  if (!box.hidden) {
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
$("tour-close").onclick = () => invoke("finish_onboarding", { launchAtLogin: true, close: true }).catch((e) => toast(String(e), true));

async function checkTry() {
  if (screen !== "tour" || tryIndex >= tries().length || tryTicked) return;
  const t = tries()[tryIndex];
  if ((t.ai && !aiReady()) || (t.voice && !voiceHere)) return;
  const done = await invoke("did_since", { since: trySince }).catch(() => []);
  const hit = done.find((d) => t.did.includes(d.what));
  if (!hit) return;
  tryTicked = true;
  const st = $("practice-status");
  st.textContent = t.actions ? `✓ It works: ${hit.text}` : "✓ It works";
  st.classList.add("ok");
  const index = tryIndex;
  setTimeout(() => {
    if (tryIndex === index) nextTry("done");
  }, 1500);
}

/** Yes turns voice actions on; Not now leaves them off. */
async function answerActions(yes) {
  if (yes && !(await changeSettings((s) => (s.actions = { ...(s.actions ?? {}), enabled: true })))) return;
  if (yes) showTry();
  else nextTry("skipped");
}

document.querySelectorAll("[data-actions]").forEach((b) =>
  b.addEventListener("click", () => answerActions(b.dataset.actions === "yes")),
);

// ---------------------------------------------------------------- startup

/** The state again: permissions change in System Settings, and setup in
 *  the background saves the services it chose (a later save from here
 *  mustn't put the old ones back). */
async function refresh() {
  try {
    state = await invoke("get_state");
  } catch {
    return; /* window closing */
  }
  if (screen === "microphone") renderMic();
  if (screen === "accessibility") renderAx();
  if (screen === "ready") renderReady();
  if (screen === "tour") {
    if (!voiceHere) voiceHere = (await invoke("reading_natural").catch(() => ({}))).downloaded ?? false;
    renderPractice();
    checkTry();
  }
}

/** Pictures drawn elsewhere may be missing: their boxes keep their size,
 *  without a broken-image mark. */
function quietMissingPictures() {
  document.querySelectorAll(".fig img").forEach((img) => {
    const missing = () => img.classList.add("missing");
    img.addEventListener("error", missing);
    if (img.complete && img.naturalWidth === 0) missing();
  });
}

(async () => {
  quietMissingPictures();
  try {
    state = await invoke("get_state");
  } catch (e) {
    toast(String(e), true);
    return;
  }
  showDictationKey(state);
  // The demo with the keys of this computer.
  if (!mac()) {
    document.querySelectorAll(".demo").forEach((demo) => {
      demo.src = "img/demo-win.svg";
      demo.alt = demo.alt.replace("the dictation key", "Right Ctrl");
    });
  }
  // Opened again while getting ready: carry on showing it.
  setup = await invoke("local_setup_status").catch(() => null);
  plan = await invoke("local_setup_plan", { speechOnly: false }).catch(() => null);

  // Closed part-way before: carry on where it was left.
  const general = state.settings.general;
  const saved = general.onboarded ? null : general.setup_step;
  if (general.setup_choice) choice = general.setup_choice;
  if (saved === "tour" && !tauri) {
    // Only the preview opens here directly (the tour is never saved).
    go("tour");
  } else if (saved && saved !== "welcome" && (MAIN.includes(saved) || saved === "key")) {
    // The downloads stop when Splashdown quits: start them again (they
    // carry on where they stopped, and anything here already counts).
    if (!setup && (choice === "local" || choice === "anthropic") && saved !== "key") {
      try {
        setup = await invoke("start_local_setup", { speechOnly: choice === "anthropic" });
      } catch {
        go("welcome");
        setInterval(refresh, 1000);
        return;
      }
    }
    go(saved);
  } else {
    go("welcome");
  }
  setInterval(refresh, 1000);
  // A download that stops sending bytes is noticed without an event.
  setInterval(() => screen === "ready" && renderReady(), 5000);
})();

// ---------------------------------------------------------------- preview

/** Sample data for the file:// preview. Only called when window.__TAURI__ is
 *  absent (see invoke in common.js). Setting up takes a few seconds.
 *  The address can set the scene: ?screen=hello&platform=windows&mic=denied
 *  &ax=on&room=speech|none&note=slow|space|downloading&stop=0.3 (the
 *  speech download holds there). */
async function mockInvoke(cmd, args) {
  const q = new URLSearchParams(location.search);
  await new Promise((r) => setTimeout(r, 50));
  mockInvoke.s ??= {
    settings: {
      speech: { provider: "auto", model: null },
      ai: { provider: "auto", model: null, base_url: null },
      general: { setup_step: q.get("screen"), setup_choice: q.get("choice") ?? "local" },
      hotkeys: { dictation: null },
    },
    keys: [{ name: "openai", source: null, hint: null }, { name: "openrouter", source: null, hint: null }, { name: "anthropic", source: null, hint: null }, { name: "custom", source: null, hint: null }],
    permissions: { accessibility: q.get("ax") === "on", microphone: q.get("mic") ?? "not-asked" },
    defaults: { local_speech: "nemotron" },
    key_options: [
      { value: "right-alt", label: q.get("platform") === "windows" ? "Right Alt" : "Right Option" },
      { value: "right-control", label: "Right Ctrl" },
      { value: "right-shift", label: "Right Shift" },
    ],
    platform: q.get("platform") ?? "macos",
  };
  const note = q.get("note");
  if (cmd === "get_state") return mockInvoke.s;
  if (cmd === "did_since") return mockInvoke.did ? [{ what: mockInvoke.did, at: Date.now(), text: "Opened Calculator" }] : [];
  if (cmd === "try_signs") {
    // A held key and a voice, now and then, to see the signs move.
    const t = Date.now() / 1000;
    const held = q.get("signs") === "on" || (q.get("signs") !== "off" && t % 6 < 3);
    return { held, held_at: held ? Date.now() : 0, heard_at: held ? Date.now() : 0, level: held ? 0.02 + 0.08 * Math.abs(Math.sin(t * 3)) : 0 };
  }
  if (cmd === "save_settings") {
    mockInvoke.s.settings = args.settings;
    return { state: mockInvoke.s, engine_error: null };
  }
  if (cmd === "request_microphone") {
    mockInvoke.s.permissions.microphone = "granted";
    return null;
  }
  if (cmd === "open_permission" && args.permission === "accessibility") {
    setTimeout(() => (mockInvoke.s.permissions.accessibility = true), 4000);
    return null;
  }
  if (cmd === "point_at_tray") return false;
  if (cmd === "local_setup_status") return null;
  if (cmd === "reading_natural") return { downloaded: !!mockInvoke.voice, mb: 68 };
  if (cmd === "download_model" && args.id === "phonon-en") {
    await new Promise((r) => setTimeout(r, 1500));
    mockInvoke.voice = true;
    return { state: mockInvoke.s, engine_error: null };
  }
  if (cmd === "local_setup_plan") {
    const room = q.get("room") ?? "all";
    return { cleanup: "local", small: false, download_mb: 3063, speech_mb: 495, ai_mb: 2500, voice_mb: 68, free_mb: room === "none" ? 900 : 80_000, too_little_space: room !== "all", room, free_up_mb: room === "none" ? 669 : 0 };
  }
  if (cmd === "start_local_setup") {
    const total = 3_063_000_000;
    const speech = 495_000_000;
    const status = { working: true, speech_ready: false, cleanup_ready: false, done: 0, total, speech_total: speech, error: null, no_room_for_ai: note === "space", slow_off: false };
    const stop = Number(q.get("stop") ?? 0);
    if (stop) {
      status.done = speech * stop;
      return status;
    }
    if (note) {
      // Speech here already: the scene is about what came after.
      Object.assign(status, { speech_ready: true, done: speech + 900_000_000, working: note === "downloading" });
      if (note === "slow") Object.assign(status, { working: false, done: total, cleanup_ready: true, slow_off: true, speed: "slow", local_ms: 9400 });
      return status;
    }
    (async () => {
      for (let i = 1; i <= 20; i++) {
        await new Promise((r) => setTimeout(r, 400));
        status.done = (total / 20) * i;
        status.speech_ready = status.done > speech;
        if (i === 20) Object.assign(status, { checking: true });
        mockEmit("setup-progress", { ...status });
      }
      await new Promise((r) => setTimeout(r, 2500));
      const ms = mockInvoke.speedMs ?? 1200;
      mockInvoke.s.settings.ai = { ...mockInvoke.s.settings.ai, provider: ms > 5000 ? "off" : "local" };
      Object.assign(status, { working: false, checking: false, cleanup_ready: true, local_ms: ms, speed: ms < 2000 ? "fast" : ms <= 5000 ? "usable" : "slow", slow_off: ms > 5000 });
      mockEmit("setup-progress", { ...status });
    })();
    return status;
  }
  return { state: mockInvoke.s, engine_error: null };
}
