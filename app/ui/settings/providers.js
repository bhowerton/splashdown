"use strict";

// Providers tab: API keys, speech-to-text (cloud, local model downloads,
// Apple on-device), cleanup & formatting, and the "Try it" box.

/** Suggestions shown in the model fields. Any model id can be typed. */
const SUGGESTIONS = {
  stt: {
    openai: ["gpt-4o-mini-transcribe", "gpt-4o-transcribe", "whisper-1"],
    openrouter: ["google/gemini-3.1-flash-lite", "google/gemini-2.5-flash-lite"],
  },
  ai: {
    openai: ["gpt-4.1-mini", "gpt-4.1-nano"],
    openrouter: [
      "google/gemini-2.5-flash-lite",
      "mistralai/mistral-small-3.2-24b-instruct",
      "openai/gpt-4.1-mini",
    ],
    anthropic: ["claude-haiku-5-5", "claude-sonnet-5-5"],
    custom: ["qwen2.5:3b", "llama3.2:3b"],
  },
};

const KEY_INFO = {
  openai: { label: "OpenAI", help: "Speech-to-text and cleanup · platform.openai.com" },
  openrouter: { label: "OpenRouter", help: "One key, many models · openrouter.ai" },
  anthropic: { label: "Anthropic", help: "Claude, for cleanup only (no speech-to-text) · platform.claude.com" },
  custom: { label: "Local server key", help: "Only if your local server needs one" },
};

function renderProviders() {
  const s = state.settings;
  const err = $("engine-error");
  err.hidden = !state.engine_error;
  err.textContent = state.engine_error ? `Dictation isn't ready: ${state.engine_error}` : "";
  renderKeys();

  renderAppleOptions();
  setValue($("stt-provider"), s.speech.provider);
  const local = s.speech.provider === "whisper-local";
  $("stt-model-row").hidden = local || s.speech.provider === "auto" || s.speech.provider === "apple";
  setValue($("stt-model"), local ? "" : s.speech.model);
  $("stt-model").placeholder = defaultModel("stt", s.speech.provider);
  fillList("stt-models", SUGGESTIONS.stt[s.speech.provider]);
  renderLocalSpeech(local);
  renderAppleSpeech();
  $("resolved-speech").textContent = resolved(state.resolved_speech);

  setValue($("ai-provider"), s.ai.provider);
  $("ai-url-row").hidden = s.ai.provider !== "custom";
  setValue($("ai-url"), s.ai.base_url);
  $("ai-model-row").hidden = ["auto", "off", "apple", "local"].includes(s.ai.provider);
  renderLocalModels();
  const aiNote = $("apple-ai-note");
  aiNote.hidden = s.ai.provider !== "apple";
  aiNote.querySelector("small").textContent = state.apple?.language_model
    ? "Free and private. Good at cleanup; less reliable than the cloud models at following longer instructions."
    : `Not available: ${state.apple?.language_model_detail ?? "needs a Mac with Apple Intelligence"}.`;
  aiNote.classList.toggle("warn-text", !state.apple?.language_model);
  // Only offered while cleanup runs locally and a cloud key exists.
  const routing = !!s.ai.cloud_for_instructions;
  $("ai-route-row").hidden = !state.can_route_instructions && !routing;
  $("ai-route").checked = routing;
  const cloud = state.resolved_instructions?.split(" · ")[0];
  $("ai-route-note").textContent = routing && !state.resolved_instructions
    ? "Not in use: needs cleanup on this computer, an OpenAI or OpenRouter key, and local-only mode off"
    : `Plain dictation stays on this computer. Spoken instructions (“format as an email”) and edits of selected text go to ${cloud ?? "your cloud provider"}, which follows them more reliably.`;
  setValue($("ai-model"), s.ai.model);
  $("ai-model").placeholder = defaultModel("ai", s.ai.provider);
  fillList("ai-models", SUGGESTIONS.ai[s.ai.provider]);
  $("resolved-ai").textContent = resolved(state.resolved_ai);
}

/** A provider the backend resolved (Rust `Result` as JSON) as display text. */
function resolved(r) {
  return "Ok" in r ? r.Ok : `not available (${r.Err})`;
}

function defaultModel(kind, provider) {
  const d = state.defaults;
  const map = kind === "stt"
    ? { openai: d.openai_stt, openrouter: d.openrouter_stt }
    : { openai: d.openai_ai, openrouter: d.openrouter_ai, anthropic: d.anthropic_ai, custom: "e.g. qwen2.5:3b" };
  const m = map[provider];
  return m ? (provider === "custom" ? m : `${m} (default)`) : "";
}

function fillList(id, items = []) {
  $(id).replaceChildren(...items.map((v) => new Option(v)));
}

// ---------------------------------------------------------------- API keys

function renderKeys() {
  const host = $("keys");
  host.replaceChildren(
    ...state.keys.map((k) => {
      const info = KEY_INFO[k.name];
      const row = document.createElement("div");
      row.className = "row key-row";
      row.innerHTML = `
        <span class="lbl">${info.label}<small>${info.help}</small></span>
        <span class="state"></span>
        <input type="password" placeholder="${k.source ? "Replace key…" : "Paste key…"}" autocomplete="off" spellcheck="false">
        <button class="btn primary">Save</button>`;
      const badge = document.createElement("span");
      if (k.source === "keychain") {
        badge.className = "badge ok";
        badge.textContent = `Saved · …${k.hint}`;
      } else if (k.source === "environment") {
        badge.className = "badge muted";
        badge.textContent = `From .env · …${k.hint}`;
        badge.title = "Set by an environment variable or .env.local, which overrides the Keychain";
      }
      const stateEl = row.querySelector(".state");
      if (k.source) stateEl.append(badge);
      if (k.source === "keychain") {
        const remove = document.createElement("button");
        remove.className = "btn danger";
        remove.textContent = "Remove";
        remove.onclick = () => removeKey(k.name);
        stateEl.append(remove);
      }
      const input = row.querySelector("input");
      input.setAttribute("aria-label", `${info.label} API key`);
      const save = row.querySelector("button.primary");
      save.setAttribute("aria-label", `Verify and save ${info.label} key`);
      const submit = () => saveKey(k.name, input, save);
      save.onclick = submit;
      input.onkeydown = (e) => e.key === "Enter" && submit();
      return row;
    })
  );
}

async function saveKey(name, input, btn) {
  const value = input.value.trim();
  if (!value) return input.focus();
  btn.disabled = true;
  btn.textContent = "Checking…";
  try {
    const r = await invoke("save_key", { name, value });
    state = r.state;
    render();
    toast(r.engine_error ? `Key saved — but dictation isn't ready: ${r.engine_error}` : "Key verified and saved");
  } catch (e) {
    toast(String(e), true);
    btn.disabled = false;
    btn.textContent = "Save";
  }
}

async function removeKey(name) {
  try {
    const r = await invoke("delete_key", { name });
    state = r.state;
    render();
    toast("Key removed");
  } catch (e) {
    toast(String(e), true);
  }
}

// ---------------------------------------------------------------- local speech

/** The speech models that run on this computer (Nemotron, Parakeet, Whisper),
 *  downloaded when chosen. */
function renderLocalSpeech(visible) {
  const host = $("local-speech-models");
  host.hidden = !visible;
  if (!visible) return host.replaceChildren();
  const selected = state.settings.speech.model ?? state.defaults.local_speech;
  host.replaceChildren(
    ...state.speech_models.map((m) => {
      const row = document.createElement("div");
      row.className = "row";
      row.dataset.model = m.id;
      const lbl = document.createElement("span");
      lbl.className = "lbl";
      lbl.textContent = m.label ?? m.id;
      const small = document.createElement("small");
      small.textContent = `~${m.mb} MB · ${m.license ?? ""}${m.id === state.defaults.local_speech ? " · recommended" : ""}`;
      lbl.append(small);
      const st = document.createElement("span");
      st.className = "state";
      row.append(lbl, st);
      if (m.id === selected) {
        const loading = m.downloaded && state.speech_loading;
        const b = document.createElement("span");
        b.className = m.downloaded && !loading ? "badge ok" : "badge warn";
        b.textContent = loading ? "Loading…" : m.downloaded ? "In use" : "Selected · not downloaded";
        st.append(b);
        if (loading) recheckLoading();
      }
      const pct = downloadPct(m.id);
      if (pct !== undefined) {
        st.replaceChildren();
        showProgress(st, pct);
      } else if (m.downloaded) {
        if (m.id !== selected) st.append(button("Use", () => chooseLocalSpeech(m.id)));
      } else {
        st.append(button("Download", () => downloadSpeechModel(m.id, m.id !== selected)));
      }
      return row;
    })
  );
}

/** While the chosen model loads, look again every second, so "Loading…"
 *  turns into "In use" when it's ready. Only this list is redrawn, so
 *  nothing being typed elsewhere in Settings is lost. */
let loadingCheck = null;
function recheckLoading() {
  if (loadingCheck) return;
  loadingCheck = setTimeout(async () => {
    loadingCheck = null;
    try {
      const fresh = await invoke("get_state");
      state.speech_loading = fresh.speech_loading;
      renderLocalSpeech(!$("local-speech-models").hidden);
    } catch {
      // Tried again on the next render.
    }
  }, 1000);
}

/** Models downloading now, with their progress (0–100): speech models and
 *  the built-in text models alike. */
const downloads = new Map();
/** The downloads this window started, which tell the user how they ended
 *  themselves. */
const mine = new Set();

/** How far a model's download has got (0–100), or undefined if it isn't
 *  downloading. One started before this window opened is in the state. */
function downloadPct(id) {
  if (downloads.has(id)) return downloads.get(id);
  const d = state.downloads?.find((d) => d.id === id);
  return d ? percent(d.done, d.total) : undefined;
}

function percent(done, total) {
  return total ? Math.floor((done / total) * 100) : 0;
}

function showProgress(st, pct) {
  st.innerHTML = `<span class="progress"><i style="width:${pct}%"></i></span><span class="mono pct">${pct}%</span>`;
}

function chooseLocalSpeech(id) {
  const s = collect();
  s.speech.model = id;
  save(s);
}

/** Downloads a speech model. The model in use stays in use until this one
 *  is ready; then, if `select`, this one takes over. */
async function downloadSpeechModel(id, select) {
  await download(id, renderLocalSpeech.bind(null, true), async (r) => {
    if (!select) return r;
    const s = r.state.settings;
    s.speech.model = id;
    return invoke("save_settings", { settings: s });
  });
}

/** Downloads a built-in text model. With "Built-in local model" or
 *  "Automatic" chosen, it's used as soon as it's here. */
async function downloadLocalModel(id) {
  await download(id, renderLocalModels, async (r) => r);
}

/** Runs a download, showing its progress, then `then` (which may save
 *  settings) with the answer. */
async function download(id, redraw, then) {
  if (downloadPct(id) !== undefined) return;
  downloads.set(id, 0);
  mine.add(id);
  redraw();
  try {
    const r = await then(await invoke("download_model", { id }));
    downloads.delete(id);
    state = r.state;
    render();
    toast(r.engine_error ? `Downloaded — but: ${r.engine_error}` : `${modelName(id)} is downloaded`);
  } catch (e) {
    downloads.delete(id);
    toast(String(e), true);
    refresh();
  } finally {
    // Its "model-downloaded" event may come after this answer.
    setTimeout(() => mine.delete(id), 2000);
  }
}

function modelName(id) {
  const m = [...state.speech_models, ...(state.local_models ?? [])].find((m) => m.id === id);
  return (m?.label ?? id).split(" · ")[0];
}

listen("model-download", (event) => {
  const { id, done, total } = event.payload;
  const pct = percent(done, total);
  const known = downloads.has(id);
  downloads.set(id, pct);
  const row = document.querySelector(`[data-model="${id}"]`);
  const bar = row?.querySelector(".progress > i");
  if (!bar) {
    // Started elsewhere (onboarding, before this window opened): show it.
    if (!known && row) render();
    return;
  }
  bar.style.width = `${pct}%`;
  const label = row.querySelector(".pct");
  if (label) label.textContent = `${pct}%`;
});

// A download this window didn't start (or can't hear back from) ended.
listen("model-downloaded", (event) => {
  const { id, error } = event.payload;
  downloads.delete(id);
  if (mine.has(id)) return;
  if (error) toast(error, true);
  refresh();
});

// ---------------------------------------------------------------- built-in local model

// The downloadable text models, shown when "Built-in local model" is chosen,
// or under "Automatic" while nothing does the cleanup yet. "Automatic" only
// uses the default (first) model, so only that one is offered there.
function renderLocalModels() {
  const host = $("local-models");
  const s = state.settings;
  const off = "Ok" in state.resolved_ai && state.resolved_ai.Ok === "Off";
  const local = s.ai.provider === "local";
  const show = local || (s.ai.provider === "auto" && off);
  if (!show || !state.local_models?.length) return host.replaceChildren();
  const selected = s.ai.model ?? state.local_models[0].id;
  const models = local ? state.local_models : state.local_models.slice(0, 1);
  host.replaceChildren(
    ...models.map((m) => {
      const row = document.createElement("div");
      row.className = "row";
      row.dataset.model = m.id;
      const ram = ((m.ram_mb ?? 3000) / 1000).toFixed(1).replace(/\.0$/, "");
      row.innerHTML = `<span class="lbl"><span></span><small>Free and private: runs on this computer, no account or key. ~${(m.mb / 1000).toFixed(1)} GB download; uses about ${ram} GB of memory while in use.</small></span><span class="state"></span>`;
      row.querySelector(".lbl > span").textContent = m.label ?? m.id;
      const st = row.querySelector(".state");
      const pct = downloadPct(m.id);
      if (pct !== undefined) {
        showProgress(st, pct);
      } else if (m.downloaded) {
        const inUse = s.ai.provider !== "auto" ? m.id === selected : !off;
        if (inUse || !local) st.append(badge(inUse ? "In use" : "Downloaded", "ok"));
        // How fast it is here, as the setup guide measured it.
        if (inUse && s.ai.local_ms) {
          const secs = s.ai.local_ms / 1000;
          const slow = s.ai.local_ms > 5000;
          row.querySelector("small").append(
            ` On this computer it tidies up a dictation in about ${secs < 10 ? secs.toFixed(1) : Math.round(secs)} s${slow ? ", which is slow: a cloud key (OpenRouter, OpenAI or Anthropic) is much quicker." : "."}`,
          );
        }
        else st.append(button("Use", () => chooseLocalModel(m.id)));
      } else {
        st.append(button("Download", () => downloadLocalModel(m.id)));
      }
      return row;
    })
  );
}

function chooseLocalModel(id) {
  const s = collect();
  s.ai.model = id;
  save(s);
}

// ---------------------------------------------------------------- Apple on-device

// Hides the Apple options off macOS; marks them when they can't be used.
function renderAppleOptions() {
  const a = state.apple;
  const stt = $("stt-provider").querySelector('option[value="apple"]');
  const ai = $("ai-provider").querySelector('option[value="apple"]');
  stt.hidden = ai.hidden = !a;
  if (!a) return;
  stt.textContent = a.speech === "unavailable"
    ? "Apple on-device (not available on this Mac)"
    : "Apple on-device (private, built into macOS)";
  ai.textContent = a.language_model
    ? "Apple on-device (private, built into macOS)"
    : "Apple on-device (not available on this Mac)";
}

// The Apple speech model's status, with a Download button when needed.
// Shown when Apple is chosen, or when "Automatic" could use it.
function renderAppleSpeech() {
  const host = $("apple-speech");
  const a = state.apple;
  const provider = state.settings.speech.provider;
  const show = a && (provider === "apple" || (provider === "auto" && a.speech !== "unavailable" && a.speech !== "ready"));
  if (!show) return host.replaceChildren();
  if (host.dataset.busy) return; // a download is running
  const row = document.createElement("div");
  row.className = "row";
  const blurb = provider === "auto"
    ? "Free and private. Used automatically once downloaded, unless you add an API key"
    : "Free and private, runs on this Mac";
  row.innerHTML = `<span class="lbl">Apple's speech model<small></small></span><span class="state"></span>`;
  const small = row.querySelector("small");
  const st = row.querySelector(".state");
  if (a.speech === "ready") {
    small.textContent = blurb;
    st.append(badge("Ready", "ok"));
  } else if (a.speech === "needs-download") {
    small.textContent = `${blurb} · downloaded from Apple`;
    st.append(button("Download", () => prepareAppleSpeech(host, st)));
  } else if (a.speech === "downloading") {
    small.textContent = "macOS is downloading it; this updates when it's done";
    st.append(badge("Downloading…", "muted"));
  } else {
    small.textContent = a.speech_detail;
    small.classList.add("warn-text");
    st.append(badge("Not available", "warn"));
  }
  host.replaceChildren(row);
}

async function prepareAppleSpeech(host, st) {
  host.dataset.busy = "1";
  st.replaceChildren(badge("Downloading…", "muted"));
  try {
    const r = await invoke("prepare_apple_speech");
    delete host.dataset.busy;
    state = r.state;
    render();
    toast(r.engine_error ? `Downloaded — but: ${r.engine_error}` : "Apple's speech model is ready");
  } catch (e) {
    delete host.dataset.busy;
    toast(String(e), true);
    refresh();
  }
}

// ---------------------------------------------------------------- try it

async function tryIt() {
  const btn = $("try-btn");
  const out = $("try-out");
  btn.disabled = true;
  btn.textContent = "Running…";
  try {
    const r = await invoke("test_ai", { text: $("try-text").value });
    out.hidden = false;
    out.innerHTML = "";
    out.append(r.output);
    const meta = document.createElement("div");
    meta.className = "meta";
    meta.textContent = [r.model ?? "AI pass off — shown unedited", `${r.millis} ms`, r.note]
      .filter(Boolean)
      .join(" · ");
    out.append(meta);
  } catch (e) {
    out.hidden = false;
    out.textContent = String(e);
  } finally {
    btn.disabled = false;
    btn.textContent = "Test";
  }
}

$("try-btn").addEventListener("click", tryIt);
$("try-text").addEventListener("keydown", (e) => e.key === "Enter" && tryIt());
