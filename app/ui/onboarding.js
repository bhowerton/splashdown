"use strict";

// First-run setup guide (needs common.js). Opened as a plain file in a
// browser it uses the sample data in mockInvoke, for design work.

const LAST = 5;
let step = 1;
let state = null;
let choice = null; // "local" | "openrouter" | "openai" | "existing"
let practiceSince = 0;
let practiceDone = false;

// ---------------------------------------------------------------- navigation

function go(n) {
  step = Math.max(1, Math.min(LAST, n));
  document.querySelectorAll(".ob-step").forEach((s) => (s.hidden = Number(s.dataset.step) !== step));
  document.querySelectorAll(".steps i").forEach((i, idx) => i.classList.toggle("on", idx < step));
  $("back").hidden = step === 1;
  if (step === 4) startPractice();
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
    if (!choice) note.textContent = "Pick one to continue";
  }
  if (step === 3 && state && !state.permissions.accessibility) {
    next.disabled = true;
    note.textContent = "Accessibility is needed for the hotkey";
  }
  if (step === 4 && !practiceDone) {
    next.textContent = "Skip";
  }
}

$("back").onclick = () => go(step - 1);
$("next").onclick = async () => {
  if (step === 2 && !(await applyChoice())) return;
  if (step === LAST) return finish();
  go(step + 1);
};

// ---------------------------------------------------------------- step 2: AI

const KEY_CHOICES = {
  openrouter: { label: "OpenRouter", site: "openrouter.ai/keys" },
  openai: { label: "OpenAI", site: "platform.openai.com/api-keys" },
  anthropic: { label: "Anthropic", site: "platform.claude.com/settings/keys" },
};

// Apple's speech model can be used here (possibly after a download).
function appleSpeech() {
  return state.apple && state.apple.speech !== "unavailable";
}

// Apple Intelligence can do the cleanup.
function appleCleanup() {
  return !!state.apple?.language_model;
}

function existingKey() {
  return state.keys.find((k) => k.source && k.name !== "custom");
}

function localModel() {
  return state.local_models?.[0];
}

// The built-in cleanup model is offered where Apple's isn't available.
let wantLocalModel = true;

function choiceReady() {
  if (choice === "local" || choice === "existing") return true;
  if (KEY_CHOICES[choice]) return state.keys.some((k) => k.name === choice && k.source);
  return false;
}

/** The row saying how speech will be recognized on this computer. */
function speechRow() {
  const row = document.createElement("div");
  row.className = "row";
  if (appleSpeech()) {
    row.innerHTML = state.apple.speech === "ready"
      ? `<span class="lbl">Apple's speech model is ready<small>Built into macOS.</small></span><span class="badge ok">Ready</span>`
      : `<span class="lbl">Downloads Apple's speech model<small>From Apple, when you click Continue.</small></span><span class="state dl-state" aria-live="polite" data-dl="apple"></span>`;
  } else {
    const id = state.defaults.local_speech;
    const model = state.speech_models.find((m) => m.id === id);
    row.innerHTML = model?.downloaded
      ? `<span class="lbl">Speech model ready<small>${model.label} is already on this computer.</small></span><span class="badge ok">Ready</span>`
      : `<span class="lbl">Downloads a speech model (~${model?.mb ?? 475} MB)<small>Happens when you click Continue.</small></span><span class="state dl-state" aria-live="polite" data-dl="${id}"></span>`;
  }
  return row;
}

/** The built-in cleanup model, as an option (checked by default). */
function localModelRow() {
  const m = localModel();
  const row = document.createElement("label");
  row.className = "row";
  const size = `~${(m.mb / 1000).toFixed(1)} GB`;
  row.innerHTML = m.downloaded
    ? `<span class="lbl">Built-in cleanup model is ready<small>Tidies “um”s and self-corrections on this computer.</small></span><span class="badge ok">Ready</span>`
    : `<span class="lbl">Also tidy up my words on this computer<small>Downloads the built-in cleanup model (${size}). Without it, your words are typed exactly as heard.</small></span><span class="state dl-state" aria-live="polite" data-dl="${m.id}"></span>`;
  if (!m.downloaded) {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "switch";
    box.checked = wantLocalModel;
    box.onchange = () => (wantLocalModel = box.checked);
    row.querySelector(".state").before(box);
  }
  return row;
}

function keyRow(name) {
  const info = KEY_CHOICES[name];
  const key = state.keys.find((k) => k.name === name);
  const row = document.createElement("div");
  row.className = "row";
  row.innerHTML = `<span class="lbl">${info.label} API key<small>Create one at ${info.site}. It's stored in your Keychain.</small></span>`;
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
  document.querySelectorAll(".card").forEach((c) => c.classList.toggle("sel", c.dataset.choice === choice));
  const detail = $("choice-detail");
  detail.replaceChildren();
  if (choice === "openrouter" || choice === "openai") {
    detail.append(keyRow(choice));
  } else if (choice === "anthropic") {
    // Claude only does the cleanup; speech stays on this computer.
    detail.append(keyRow("anthropic"), speechRow());
  } else if (choice === "local") {
    detail.append(speechRow());
    if (appleCleanup()) {
      const row = document.createElement("div");
      row.className = "row";
      row.innerHTML = `<span class="lbl">Cleanup uses Apple Intelligence<small>Also on this Mac.</small></span><span class="badge ok">Ready</span>`;
      detail.append(row);
    } else if (localModel()) {
      detail.append(localModelRow());
    }
  }
  updateFooter();
}

document.querySelectorAll(".card").forEach((c) =>
  c.addEventListener("click", () => {
    choice = c.dataset.choice;
    renderChoice();
  })
);

/** Runs a download step, showing progress in its row. Returns false if it
 *  failed (the error is shown). */
async function downloadStep(id, command, args) {
  const st = document.querySelector(`.dl-state[data-dl="${id}"]`);
  if (st) st.innerHTML = `<span class="progress wide${id === "apple" ? " indeterminate" : ""}"><i></i></span>`;
  try {
    const r = await invoke(command, args);
    state = r.state;
    return true;
  } catch (e) {
    toast(String(e), true);
    return false;
  }
}

/** Gets speech recognition on this computer ready; returns its settings. */
async function prepareLocalSpeech() {
  if (appleSpeech()) {
    if (state.apple.speech !== "ready" && !(await downloadStep("apple", "prepare_apple_speech", {}))) return null;
    return { provider: "apple", model: null };
  }
  const id = state.defaults.local_speech;
  const model = state.speech_models.find((m) => m.id === id);
  if (!model?.downloaded && !(await downloadStep(id, "download_model", { id }))) return null;
  return { provider: "whisper-local", model: id };
}

/** Saves the chosen setup. Returns false to stay on the step. */
async function applyChoice() {
  const settings = structuredClone(state.settings);
  $("next").disabled = true;
  const fail = () => {
    $("next").disabled = false;
    return false;
  };
  if (choice === "local") {
    const speech = await prepareLocalSpeech();
    if (!speech) return fail();
    settings.speech = speech;
    // Keep cleanup on a key that already exists; otherwise on this computer.
    if (!existingKey()) {
      let ai = "off";
      if (appleCleanup()) {
        ai = "apple";
      } else if (localModel() && (localModel().downloaded || wantLocalModel)) {
        const m = localModel();
        if (!m.downloaded && !(await downloadStep(m.id, "download_model", { id: m.id }))) return fail();
        ai = "local";
      }
      settings.ai = { ...settings.ai, provider: ai, model: null };
    }
  } else if (choice === "anthropic") {
    const speech = await prepareLocalSpeech();
    if (!speech) return fail();
    settings.speech = speech;
    settings.ai = { ...settings.ai, provider: "anthropic", model: null };
  } else {
    settings.speech = { provider: "auto", model: null };
    settings.ai = { ...settings.ai, provider: "auto", model: null };
  }
  try {
    const r = await invoke("save_settings", { settings });
    state = r.state;
  } catch (e) {
    toast(String(e), true);
    return fail();
  }
  return true;
}

listen("model-download", (event) => {
  const { id, done, total } = event.payload;
  const bar = document.querySelector(`.dl-state[data-dl="${id}"] .progress > i`);
  if (bar && total) bar.style.width = `${Math.floor((done / total) * 100)}%`;
});

// ---------------------------------------------------------------- step 3: permissions

function renderPermissions() {
  renderPermissionRows(state.permissions, "", "step-dot");
  if (step === 3) updateFooter();
}

// ---------------------------------------------------------------- step 4: practice

function startPractice() {
  practiceSince = Date.now();
  setTimeout(() => $("practice").focus(), 50);
}

async function checkPractice() {
  if (step !== 4 || practiceDone) return;
  const last = await invoke("last_result").catch(() => null);
  if (last && last.at >= practiceSince) {
    practiceDone = true;
    const st = $("practice-status");
    st.textContent = `✓ It works — heard “${last.raw || last.text}”`;
    st.classList.add("ok");
    $("tips").hidden = false;
    updateFooter();
  }
}

// ---------------------------------------------------------------- step 5: done

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
  checkPractice();
}

(async () => {
  state = await invoke("get_state");
  showDictationKey(state);

  const free = document.querySelector('.card[data-choice="local"] p');
  if (appleSpeech() && appleCleanup()) {
    free.textContent = "Uses the speech recognition and Apple Intelligence built into your Mac. Nothing leaves it.";
  } else if (localModel()) {
    free.textContent = "Speech recognition and cleanup run on this computer, with free downloads. Nothing leaves it.";
  }

  // A key that's already set up (e.g. from .env.local) is offered as-is.
  const existing = existingKey();
  if (existing) {
    const name = KEY_CHOICES[existing.name]?.label ?? existing.name;
    const card = document.createElement("button");
    card.className = "card";
    card.dataset.choice = "existing";
    card.innerHTML = `<span class="ic">✓</span><b>Use my ${name} key</b><p>Already set up (…${existing.hint}).</p><span class="badge ok">Ready</span>`;
    card.addEventListener("click", () => {
      choice = "existing";
      renderChoice();
    });
    $("ai-cards").prepend(card);
    choice = "existing";
  }
  renderChoice();
  renderPermissions();
  go(1);
  setInterval(poll, 1500);
})();

// ---------------------------------------------------------------- preview

/** Sample data for the file:// preview. Only called when window.__TAURI__ is
 *  absent (see invoke in common.js). */
async function mockInvoke(cmd) {
  await new Promise((r) => setTimeout(r, 200));
  mockInvoke.s ??= {
    settings: { speech: { provider: "auto", model: null }, ai: { provider: "auto", model: null, base_url: null }, general: {}, hotkeys: { dictation: null } },
    keys: [{ name: "openai", source: null, hint: null }, { name: "openrouter", source: "environment", hint: "9f2c" }, { name: "anthropic", source: null, hint: null }, { name: "custom", source: null, hint: null }],
    permissions: { accessibility: false, microphone: "not-asked" },
    defaults: { local_speech: "nemotron" },
    speech_models: [{ id: "nemotron", mb: 495, downloaded: true, label: "Nemotron 3.5" }],
    local_models: [{ id: "qwen3-4b", mb: 2500, downloaded: false, label: "Qwen3 4B · best quality" }],
    key_options: [{ value: "right-alt", label: "Right Option" }],
    platform: "macos",
  };
  if (cmd === "get_state") return mockInvoke.s;
  if (cmd === "last_result") return null;
  return { state: mockInvoke.s, engine_error: null };
}
