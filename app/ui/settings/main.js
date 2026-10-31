"use strict";

// Settings window core, loaded last: the backend state, tab switching, render()
// (which calls each tab's render function), collect()/save(), and startup.

/** The snapshot from get_state; every tab renders from it. */
let state = null;

// ---------------------------------------------------------------- tabs

function showTab(tab = (location.hash || "#general").slice(1)) {
  if (tab === "history") loadHistory();
  if (tab === "insights") loadInsights();
  if (tab === "presets") loadRecentApps();
  if (tab === "general") loadMicrophones();
  if (tab === "words") loadSuggestions();
  document.querySelectorAll(".tab").forEach((s) => (s.hidden = s.id !== `tab-${tab}`));
  document.querySelectorAll(".side a[data-tab]").forEach((a) => {
    const on = a.dataset.tab === tab;
    a.classList.toggle("on", on);
    if (on) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  });
  document.querySelector(`#tab-${tab} h1`)?.setAttribute("tabindex", "-1");
}
document.querySelectorAll(".side a[data-tab]").forEach((a) => {
  a.addEventListener("click", (e) => {
    e.preventDefault();
    try {
      history.replaceState(null, "", `#${a.dataset.tab}`);
    } catch {
      /* not every page origin allows it; the tab still switches */
    }
    showTab(a.dataset.tab);
  });
});

// ---------------------------------------------------------------- render

/** Sets a control's value unless the user is editing it right now. */
function setValue(el, value) {
  if (document.activeElement !== el) el.value = value ?? "";
}

function render() {
  renderGeneral();
  renderProviders();
  renderPermissions();
  renderPresets();
  renderPrivacy();
  renderSnippets();
  renderReplacements();
  renderTransforms();
  renderSpending();
  showDictationKey(state);

  // About
  $("about-version").textContent = state.version;
  $("side-version").textContent = `Version ${state.version}`;
  $("about-platform").textContent = state.platform;
  $("about-config").textContent = state.config_path;
  renderUpdate();
}

/** Settings → About → Updates, from state.update (see updates.rs). */
function renderUpdate() {
  const u = state.update ?? {};
  const busy = u.checking || u.installing;
  let title;
  let detail = "";
  if (u.installing) {
    title = `Installing version ${u.available}…`;
    detail = u.progress != null && u.progress < 100 ? `Downloading: ${u.progress}%` : "Splashdown restarts when it's done";
  } else if (u.available) {
    title = `Version ${u.available} is available`;
    detail = `You have ${state.version}. Nothing is installed until you click, then Splashdown restarts.`;
  } else if (u.checking) {
    title = "Checking for updates…";
  } else if (u.last_checked) {
    title = "Splashdown is up to date";
    detail = `Last checked ${new Date(u.last_checked).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}`;
  } else {
    title = "Updates";
    detail = state.settings.general.check_for_updates === false ? "Automatic checks are off (General)" : "Not checked yet";
  }
  $("upd-title").textContent = title;
  const small = $("upd-detail");
  small.textContent = u.error || detail;
  small.classList.toggle("warn-text", !!u.error);
  $("upd-check").disabled = busy;
  $("upd-check").textContent = u.checking ? "Checking…" : "Check now";
  $("upd-install").hidden = !u.available;
  $("upd-install").disabled = busy;
  $("upd-notes-row").hidden = !(u.available && u.notes);
  $("upd-notes").textContent = u.notes ?? "";
}

// ---------------------------------------------------------------- save

/** The settings as shown in the form (every tab), ready for save_settings. */
function collect() {
  const s = structuredClone(state.settings);
  s.hotkeys.dictation = $("hk-dictation").dataset.value;
  s.hotkeys.instruction = $("hk-instruction").dataset.value;
  // A letter; H (the default) is saved as unset.
  const letter = $("hk-hands-free").value.trim().toLowerCase();
  s.hotkeys.hands_free = /^[a-z]$/.test(letter) && letter !== "h" ? letter : null;
  s.hotkeys.double_tap = $("hk-double-tap").checked ? true : null;
  s.general.user_name = $("gen-name").value.trim() || null;
  s.general.language = $("gen-language").value || null;
  s.general.vocabulary = $("gen-vocab").value.split(/[,\n]/).map((v) => v.trim()).filter(Boolean);
  s.general.live_preview = $("gen-live").checked ? null : false;
  s.general.keep_history = $("hist-keep").checked ? true : null;
  s.general.keep_audio = $("hist-audio").checked ? true : null;
  s.general.overlay_near_cursor = $("gen-near-cursor").checked ? null : false;
  s.general.whisper_mode = $("gen-whisper").checked ? true : null;
  s.general.languages = mixedLanguages();
  s.general.mini_bar = $("gen-mini-bar").checked ? true : null;
  s.general.noise_reduction = $("gen-noise").checked ? true : null;
  s.general.history_days = Number($("hist-days").value) || null;
  s.general.other_audio = $("gen-other-audio").value;
  s.general.insights = $("ins-keep").checked ? null : false;
  s.general.learn_vocabulary = $("gen-learn").checked ? null : false;
  s.general.check_for_updates = $("gen-updates").checked ? null : false;
  s.general.sounds = $("gen-sounds").checked ? null : false;
  s.general.sound_volume = Number($("gen-volume").value) / 100;
  s.general.microphone = $("gen-mic").value || null;
  s.general.budget = collectBudget();
  s.general.local_only = $("pv-local").checked;
  s.general.share_app_name = $("pv-appname").checked ? null : false;
  s.general.share_recent = $("pv-recent").checked ? null : false;

  const sttProvider = $("stt-provider").value;
  if (sttProvider !== s.speech.provider) {
    s.speech.model = null; // a new provider starts from its default
  } else if (sttProvider !== "whisper-local") {
    s.speech.model = $("stt-model").value.trim() || null;
  }
  s.speech.provider = sttProvider;

  const aiProvider = $("ai-provider").value;
  if (aiProvider !== s.ai.provider) {
    s.ai.model = null;
  } else {
    s.ai.model = $("ai-model").value.trim() || null;
  }
  s.ai.provider = aiProvider;
  s.ai.base_url = $("ai-url").value.trim() || null;
  s.ai.cloud_for_instructions = $("ai-route").checked;
  return s;
}

async function save(settings = collect()) {
  // Matching keys are already flagged next to them (renderHotkeyTip);
  // saving would only be refused.
  if (settings.hotkeys.dictation === settings.hotkeys.instruction) return;
  try {
    const r = await invoke("save_settings", { settings });
    state = r.state;
    render();
    if (r.engine_error) toast(`Saved — but dictation isn't ready: ${r.engine_error}`, true);
    else toast("Saved");
  } catch (e) {
    toast(String(e), true);
    render();
  }
}

/** Re-reads everything (used after errors). */
async function refresh() {
  state = await invoke("get_state");
  render();
}

// ---------------------------------------------------------------- wiring

// Any control marked data-setting saves the whole form when it changes.
document.querySelectorAll("[data-setting]").forEach((el) => {
  el.addEventListener("change", () => save());
});
$("run-setup").addEventListener("click", () => invoke("open_onboarding"));
// The log never contains dictated text, so it's safe to attach to a bug report.
$("show-log").addEventListener("click", () => invoke("show_log_file").catch((e) => toast(String(e), true)));

// Updates: the Rust side sends the new status whenever it changes.
listen("update-status", (event) => {
  if (!state) return;
  state.update = event.payload;
  renderUpdate();
});
$("upd-check").addEventListener("click", async () => {
  try {
    const version = await invoke("check_for_updates");
    if (!version) toast("Splashdown is up to date");
  } catch {
    /* the reason is shown in the Updates row */
  }
  await refresh();
});
// Only ever on this click: downloads, installs and relaunches.
$("upd-install").addEventListener("click", () => invoke("install_update").catch((e) => toast(String(e), true)));
$("send-feedback").addEventListener("click", () => invoke("send_feedback").catch((e) => toast(String(e), true)));

// About → Settings: export writes config.toml (no keys) to Downloads; import
// reads a file chosen here and hands its text to the app.
$("cfg-export").addEventListener("click", async () => {
  try {
    const path = await invoke("export_settings");
    toast(`Saved to ${path.split(/[\\/]/).pop()} in Downloads`);
  } catch (e) {
    toast(String(e), true);
  }
});
$("cfg-import").addEventListener("click", () => $("cfg-import-file").click());
$("cfg-import-file").addEventListener("change", async () => {
  const input = $("cfg-import-file");
  const file = input.files[0];
  input.value = ""; // so choosing the same file again still fires
  if (!file) return;
  try {
    const text = await file.text();
    const found = await invoke("check_settings_file", { text });
    const count = (n, one) => `${n} ${one}${n === 1 ? "" : "s"}`;
    const presets = found.presets == null ? "the built-in presets" : count(found.presets, "preset");
    const sure = await askConfirm(`Replace your settings with “${file.name}”?`, {
      action: "Import",
      detail: `Every setting here is replaced: ${presets}, ${count(found.snippets, "snippet")}, ${count(found.vocabulary, "vocabulary word")}. Your API keys and history stay as they are.`,
    });
    if (!sure) return;
    const r = await invoke("import_settings", { text });
    state = r.state;
    render();
    if (r.engine_error) toast(`Imported — but dictation isn't ready: ${r.engine_error}`, true);
    else toast("Settings imported");
  } catch (e) {
    toast(String(e), true);
  }
});
// The menu bar opens a tab here ("Update to…" → About).
listen("show-tab", (event) => showTab(event.payload));

(async () => {
  showTab();
  await refresh();
  loadSuggestions();
  loadMicrophones();
  // Until setup is finished, start on the Permissions tab.
  if (!location.hash && !permissionsDone()) showTab("permissions");
  setInterval(pollPermissions, 2000);
})();
