"use strict";

// Presets tab. Two views:
//  - Presets: the list (in priority order: drag to reorder; 1–9 pick them
//    while recording) and an editor in three parts — Where (apps and
//    websites), How it writes (AI, tone, instructions, Try it) and Extras.
//  - By app: recently used apps and websites with the preset each gets.
// Saved as settings.presets.

/** Target languages offered for a preset's "Translate into". */
const TRANSLATE_LANGUAGES = ["English", "Spanish", "French", "German", "Italian", "Portuguese", "Dutch", "Japanese", "Chinese", "Korean"];

/** Spoken languages a preset can override (the same list as General). */
const SPOKEN_LANGUAGES = [
  ["en", "English"], ["es", "Spanish"], ["fr", "French"], ["de", "German"], ["it", "Italian"],
  ["pt", "Portuguese"], ["nl", "Dutch"], ["ja", "Japanese"], ["zh", "Chinese"], ["ko", "Korean"],
];

/** Tones (settings `tone`), each with the same sentence written that way. */
const TONES = [
  [null, "Neutral", "Sounds good, I'll send the report on Friday."],
  ["formal", "Formal", "That sounds good. I will send the report on Friday."],
  ["casual", "Casual", "Sounds good! I'll send the report Friday."],
  ["very-casual", "Very casual", "sounds good, ill send it friday"],
];

/** Starting points for "New preset". */
const TEMPLATES = [
  { name: "Blank", about: "Start from nothing" },
  { name: "Email", about: "Short paragraphs, no added greetings", instructions: "Use short paragraphs. Only add a greeting or sign-off if I say one or ask for it. Keep my wording." },
  { name: "Chat", about: "Casual, no greetings or sign-offs", tone: "casual", instructions: "No greeting or sign-off. Short paragraphs." },
  { name: "Texting", about: "Very casual, and sends right away", tone: "very-casual", auto_send: true },
  { name: "Coding agent", about: "Clean prompts with @file mentions", file_tags: true, clipboard_context: true, instructions: "This is a request to an AI coding assistant. Keep technical terms, file names, commands and code exactly as said; format code names as code." },
  { name: "Prompt for an AI", about: "Turns rambling into a clear request", instructions: "Turn what I say into a clear request for an AI assistant: the goal first, then context and constraints as short bullet points. Keep every detail I mention, and don't answer the request yourself." },
  { name: "Notes", about: "Markdown headings and lists", instructions: "Use Markdown lists and headings when I describe structure. Keep my wording." },
  { name: "Social post", about: "A punchy first line, short sentences", tone: "casual", instructions: "Write it as a social media post: a punchy first line, short sentences. No hashtags unless I say them." },
  { name: "Formal letter", about: "Polished, full paragraphs", tone: "formal", instructions: "Use full paragraphs. Add a greeting and sign-off only if I say them." },
  { name: "Translate to Spanish", about: "Types what you say in Spanish", translate_to: "Spanish" },
];

let selectedPreset = "default";
let presetView = "presets";
let recentApps = [];
/** Names and icons by app id (from app_info), filled in as they arrive. */
const appInfo = new Map();
const appInfoAsked = new Set();

// ---------------------------------------------------------------- data

async function loadRecentApps() {
  try {
    recentApps = await invoke("get_recent_apps");
  } catch {
    recentApps = [];
  }
  renderPresets();
}

/** Looks up names and icons for any app ids not seen yet, then redraws. */
function requestAppInfo(ids) {
  const fresh = [...new Set(ids)].filter((id) => id && !appInfoAsked.has(id));
  if (!fresh.length) return;
  fresh.forEach((id) => appInfoAsked.add(id));
  invoke("app_info", { ids: fresh })
    .then((infos) => {
      infos.forEach((i) => appInfo.set(i.id, i));
      renderPresets();
    })
    .catch(() => {});
}

/** The ids of every app rule and recent app, for requestAppInfo. */
function appIds() {
  return [
    ...state.presets.flatMap((p) => p.rules.map((r) => r.app).filter(Boolean)),
    ...recentApps.map((a) => a.id),
  ];
}

/** "Mail" for com.apple.mail if it's installed; otherwise a readable
 *  name from the id ("org.whispersystems.signal-desktop" → "Signal"). */
function appName(id) {
  return appInfo.get(id)?.name ?? recentApps.find((a) => a.id === id)?.name ?? readableId(id);
}

function readableId(id) {
  if (id.endsWith("*")) return id;
  const last = id.replace(/\.exe$/i, "").split(".").pop();
  const words = last.replace(/-desktop$/i, "").split(/[-_]+/).filter(Boolean);
  const name = words.map((w) => w[0].toUpperCase() + w.slice(1)).join(" ");
  return name || id;
}

/** An app's icon, or a letter badge when it has none here. */
function appIcon(host, id, name) {
  const info = appInfo.get(id);
  if (info?.icon) {
    const img = document.createElement("img");
    img.src = info.icon;
    img.alt = "";
    host.append(img);
  } else {
    const badge = document.createElement("span");
    badge.className = "letter-badge";
    badge.textContent = (name || "?")[0].toUpperCase();
    host.append(badge);
  }
}

// The same matching as splashdown_config::match_preset, for showing which
// preset an app gets.
function ruleMatches(r, ctx) {
  const set = (v) => v != null && v !== "";
  if (![r.app, r.title, r.url].some(set)) return false;
  const lower = (s) => (s ?? "").toLowerCase();
  const appOk =
    !set(r.app) ||
    [ctx.app_id, ctx.app_name].some((have) =>
      r.app.endsWith("*") ? lower(have).startsWith(lower(r.app.slice(0, -1))) : lower(have) === lower(r.app)
    );
  const contains = (want, have) => !set(want) || (have != null && lower(have).includes(lower(want)));
  return appOk && contains(r.title, ctx.title) && contains(r.url, ctx.url);
}

function matchPreset(presets, ctx) {
  return (
    presets.find((p) => p.id !== "default" && p.rules.some((r) => ruleMatches(r, ctx))) ??
    presets.find((p) => p.id === "default") ??
    presets[0]
  );
}

// ---------------------------------------------------------------- render

function renderPresets() {
  requestAppInfo(appIds());
  document.querySelectorAll(".view-switch [data-view]").forEach((b) => {
    const on = b.dataset.view === presetView;
    b.classList.toggle("on", on);
    b.setAttribute("aria-selected", String(on));
  });
  $("presets-view").hidden = presetView !== "presets";
  $("apps-view").hidden = presetView !== "apps";
  if (presetView === "apps") renderAppsView();
  else renderPresetsView();
}

function renderPresetsView() {
  const presets = state.presets;
  if (!presets.some((p) => p.id === selectedPreset)) selectedPreset = presets[0]?.id;
  $("preset-list").replaceChildren(...presets.map(presetListItem), button("+ New preset", chooseTemplate, "preset-item preset-add"));
  const p = presets.find((x) => x.id === selectedPreset);
  $("preset-edit").replaceChildren(...(p ? presetEditor(p) : []));
}

/** One line in the list: number, name, where it's used. Draggable. */
function presetListItem(p, i) {
  const item = document.createElement("button");
  item.className = `preset-item${p.id === selectedPreset ? " on" : ""}`;
  item.setAttribute("role", "listitem");
  item.draggable = true;
  item.dataset.id = p.id;
  const num = document.createElement("span");
  num.className = "num";
  num.textContent = i < 9 ? String(i + 1) : "";
  const text = document.createElement("span");
  text.className = "pi-text";
  const name = document.createElement("span");
  name.className = "pi-name";
  name.textContent = p.name || "Untitled";
  const where = document.createElement("small");
  where.textContent = whereSummary(p);
  text.append(name, where);
  item.append(num, text);
  item.title = `Drag to change the order. ${state.platform === "macos" ? "⌥↑/↓" : "Alt+↑/↓"} also moves it.`;
  item.setAttribute("aria-label", `${i + 1}. ${p.name}: ${whereSummary(p)}`);
  item.onclick = () => {
    selectedPreset = p.id;
    renderPresets();
  };
  // Keyboard reordering.
  item.onkeydown = (e) => {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    movePreset(p.id, i + (e.key === "ArrowUp" ? -1 : 1));
    requestAnimationFrame(() => document.querySelector(`.preset-item[data-id="${CSS.escape(p.id)}"]`)?.focus());
  };
  item.ondragstart = (e) => {
    e.dataTransfer.setData("text/plain", p.id);
    e.dataTransfer.effectAllowed = "move";
    item.classList.add("dragging");
  };
  item.ondragend = () => item.classList.remove("dragging");
  item.ondragover = (e) => {
    e.preventDefault();
    item.classList.add("drop-target");
  };
  item.ondragleave = () => item.classList.remove("drop-target");
  item.ondrop = (e) => {
    e.preventDefault();
    item.classList.remove("drop-target");
    const id = e.dataTransfer.getData("text/plain");
    if (id && id !== p.id) movePreset(id, i);
  };
  return item;
}

/** "Mail, Gmail +6", "Everywhere else", or "No apps yet". */
function whereSummary(p) {
  if (p.id === "default") return "Everywhere else";
  const names = p.rules
    .map((r) => (r.app && !isOtherPlatform(r.app) ? appName(r.app) : r.url || (r.title ? `“${r.title}”` : null)))
    .filter(Boolean)
    .filter((n, i, all) => all.indexOf(n) === i);
  if (!names.length) return p.shortcut ? "By shortcut only" : "No apps yet";
  return names.length > 2 ? `${names.slice(0, 2).join(", ")} +${names.length - 2}` : names.join(", ");
}

/** A Windows executable on a Mac (or a bundle ID on Windows): kept so the
 *  same settings work on both, but not shown as an app here. */
function isOtherPlatform(app) {
  const exe = /\.exe$/i.test(app);
  return state.platform === "macos" ? exe : !exe && app.includes(".");
}

function presetEditor(p) {
  const isDefault = p.id === "default";
  const head = document.createElement("div");
  head.className = "pe-head";
  const name = document.createElement("input");
  name.type = "text";
  name.className = "pe-name";
  name.value = p.name;
  name.setAttribute("aria-label", "Preset name");
  name.onchange = () => updatePreset(p.id, (x) => (x.name = name.value.trim() || x.name));
  head.append(name);
  head.append(
    button("Share…", async () => {
      try {
        const path = await invoke("export_preset", { preset: p });
        toast(`Saved to ${path.split(/[\\/]/).pop()} in Downloads`);
      } catch (e) {
        toast(String(e), true);
      }
    })
  );
  if (!isDefault) {
    head.append(
      button("Delete", async () => {
        if (!(await askConfirm(`Delete the “${p.name}” preset?`, { detail: "Apps that used it go back to the Default preset." }))) return;
        selectedPreset = "default";
        savePresets(state.presets.filter((x) => x.id !== p.id));
      }, "btn danger")
    );
  }
  return [head, whereGroup(p), writesGroup(p), extrasGroup(p)];
}

function presetGroup(title, ...children) {
  const g = document.createElement("div");
  g.className = "group";
  const h = document.createElement("div");
  h.className = "gh";
  h.textContent = title;
  g.append(h, ...children);
  return g;
}

function presetRow(label, hint, control, cls = "row") {
  const r = document.createElement(control && control.tagName === "INPUT" && control.type === "checkbox" ? "label" : "div");
  r.className = cls;
  const lbl = document.createElement("span");
  lbl.className = "lbl";
  lbl.textContent = label;
  if (hint) {
    const s = document.createElement("small");
    s.textContent = hint;
    lbl.append(s);
  }
  r.append(lbl);
  if (control) r.append(control);
  return r;
}

function presetSwitch(checked, onchange, disabled = false) {
  const s = document.createElement("input");
  s.type = "checkbox";
  s.className = "switch";
  s.checked = checked;
  s.disabled = disabled;
  s.onchange = () => onchange(s.checked);
  return s;
}

function presetSelect(options, value, onchange, label) {
  const s = document.createElement("select");
  s.append(...options.map(([v, l]) => new Option(l, v ?? "")));
  s.value = value ?? "";
  s.onchange = () => onchange(s.value || null);
  if (label) s.setAttribute("aria-label", label);
  return s;
}

// ---------------------------------------------------------------- Where

function whereGroup(p) {
  if (p.id === "default") {
    return presetGroup("Where", presetRow("Everywhere else", "Used wherever no other preset matches, and when no app is known."));
  }
  const body = document.createElement("div");
  body.className = "row column";
  const chips = document.createElement("div");
  chips.className = "chips";
  p.rules.forEach((r, i) => {
    if (r.app && isOtherPlatform(r.app)) return;
    chips.append(ruleChip(p, r, i));
  });
  chips.append(addRuleControl(p));
  body.append(chips);
  const hidden = p.rules.filter((r) => r.app && isOtherPlatform(r.app)).length;
  if (hidden) {
    const note = document.createElement("small");
    note.className = "muted";
    note.textContent = `Also ${plural(hidden, `${state.platform === "macos" ? "Windows" : "Mac"} app`)}, kept for that platform.`;
    body.append(note);
  }
  const conflicts = conflictNotes(p);
  if (conflicts.length) {
    const warn = document.createElement("small");
    warn.className = "warn-text";
    warn.textContent = conflicts.join(" ");
    body.append(warn);
  }
  const help = document.createElement("small");
  help.className = "muted";
  help.textContent = "Websites match their address, e.g. mail.google.com. If two presets claim the same app, the one higher in the list wins.";
  body.append(help);
  return presetGroup("Where", body);
}

/** A rule as a chip: the app's icon and name, a website, or a title. */
function ruleChip(p, r, i) {
  const chip = document.createElement("span");
  chip.className = "chip rule-chip";
  const kind = ["app", "url", "title"].filter((k) => r[k] != null);
  if (kind.length === 1 && r[kind[0]] === "") {
    // A rule being typed.
    const input = document.createElement("input");
    input.type = "text";
    input.spellcheck = false;
    input.placeholder = { app: "App ID, e.g. com.linear", url: "Address, e.g. mail.google.com", title: "Window title contains…" }[kind[0]];
    input.setAttribute("aria-label", input.placeholder);
    input.onchange = () => updatePreset(p.id, (x) => (x.rules[i] = { app: null, url: null, title: null, [kind[0]]: input.value.trim() }));
    input.onblur = () => {
      if (!input.value.trim()) updatePreset(p.id, (x) => x.rules.splice(i, 1));
    };
    setTimeout(() => input.focus(), 0);
    chip.append(input);
  } else {
    const icon = document.createElement("span");
    icon.className = "chip-icon";
    icon.setAttribute("aria-hidden", "true");
    const label = document.createElement("span");
    if (r.app && kind.length === 1) {
      const info = appInfo.get(r.app);
      label.textContent = appName(r.app);
      appIcon(icon, r.app, label.textContent);
      if (!info?.name) {
        chip.classList.add("missing");
        chip.title = r.app.endsWith("*") ? `Every app whose ID starts with ${r.app.slice(0, -1)}` : `${r.app}: not installed on this computer`;
      }
    } else if (r.url && kind.length === 1) {
      icon.textContent = "🌐";
      label.textContent = r.url;
    } else if (r.title && kind.length === 1) {
      icon.textContent = "▭";
      label.textContent = `Title: “${r.title}”`;
    } else {
      label.textContent = ["app", "url", "title"].filter((k) => r[k]).map((k) => `${k}: ${r[k]}`).join(" and ");
    }
    chip.append(icon, label);
  }
  const x = document.createElement("button");
  x.className = "chip-x";
  x.textContent = "×";
  x.setAttribute("aria-label", `Remove ${chip.textContent}`);
  x.onclick = () => updatePreset(p.id, (y) => y.rules.splice(i, 1));
  chip.append(x);
  return chip;
}

function addRuleControl(p) {
  const sel = document.createElement("select");
  sel.className = "chip-add";
  sel.setAttribute("aria-label", "Add an app or website");
  sel.append(new Option("+ Add app or website", ""));
  const have = new Set(p.rules.flatMap((r) => [r.app, r.url].filter(Boolean)));
  const apps = document.createElement("optgroup");
  apps.label = "Recently used";
  const seen = new Set();
  recentApps.forEach((a) => {
    if (!have.has(a.id) && !seen.has(a.id)) {
      seen.add(a.id);
      apps.append(new Option(appName(a.id), `app:${a.id}`));
    }
    if (a.site && !have.has(a.site) && !seen.has(a.site)) {
      seen.add(a.site);
      apps.append(new Option(`${a.site} (website)`, `url:${a.site}`));
    }
  });
  const other = document.createElement("optgroup");
  other.label = "Other";
  other.append(new Option("Another app…", "app:"), new Option("Another website…", "url:"), new Option("Window title contains…", "title:"));
  if (apps.children.length) sel.append(apps);
  sel.append(other);
  sel.onchange = () => {
    const [kind, ...rest] = sel.value.split(":");
    if (!kind) return;
    updatePreset(p.id, (x) => x.rules.push({ app: null, url: null, title: null, [kind]: rest.join(":") }));
  };
  return sel;
}

/** Apps this preset claims that a preset higher in the list gets first. */
function conflictNotes(p) {
  const index = state.presets.findIndex((x) => x.id === p.id);
  const notes = [];
  for (const r of p.rules) {
    const what = r.app || r.url;
    if (!what || (r.app && isOtherPlatform(r.app))) continue;
    const ctx = { app_id: r.app ?? "", app_name: "", url: r.url, title: r.title };
    const winner = state.presets.slice(0, index).find((x) => x.id !== "default" && x.rules.some((y) => ruleMatches(y, ctx)));
    if (winner) notes.push(`${r.app ? appName(r.app) : r.url} is also in “${winner.name}”, which is higher in the list, so that one is used there.`);
  }
  return notes;
}

// ---------------------------------------------------------------- How it writes

function writesGroup(p) {
  const rows = [];
  rows.push(cleanupRow(p));

  const tones = document.createElement("div");
  tones.className = "tones";
  tones.setAttribute("role", "radiogroup");
  tones.setAttribute("aria-label", "Tone");
  for (const [value, label, example] of TONES) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = `tone${(p.tone ?? null) === value ? " on" : ""}`;
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String((p.tone ?? null) === value));
    b.disabled = !p.ai;
    const t = document.createElement("b");
    t.textContent = label;
    const ex = document.createElement("span");
    ex.textContent = example;
    b.append(t, ex);
    b.onclick = () => updatePreset(p.id, (x) => (x.tone = value));
    tones.append(b);
  }
  const toneRow = document.createElement("div");
  toneRow.className = "row column";
  const toneLbl = document.createElement("span");
  toneLbl.className = "lbl";
  toneLbl.textContent = "Tone";
  toneRow.append(toneLbl, tones);
  rows.push(toneRow);

  const instr = document.createElement("textarea");
  instr.rows = 3;
  instr.value = p.instructions;
  instr.placeholder = "e.g. “No sign-off” or “Use bullet points for lists”";
  instr.disabled = !p.ai;
  instr.setAttribute("aria-label", "More instructions for the AI");
  instr.onchange = () => updatePreset(p.id, (x) => (x.instructions = instr.value.trim()));
  const instrRow = document.createElement("div");
  instrRow.className = "row column";
  const instrLbl = document.createElement("span");
  instrLbl.className = "lbl";
  instrLbl.innerHTML = "More instructions<small>Anything else the AI should do in this preset</small>";
  instrRow.append(instrLbl, instr);
  rows.push(instrRow);

  rows.push(tryRow(p));
  return presetGroup("How it writes", ...rows);
}

/** Cleanup levels: None turns the AI pass off for this preset. */
const CLEANUP_LEVELS = [
  ["none", "None", "Exactly what you said, as the speech model heard it"],
  ["light", "Light", "Fillers and punctuation only; every other word stays"],
  ["medium", "Medium", "Cleans up and applies corrections, keeping your wording"],
  ["high", "High", "Also tightens rambling and fixes grammar"],
];

function cleanupRow(p) {
  const current = !p.ai ? "none" : p.cleanup ?? "medium";
  const seg = document.createElement("span");
  seg.className = "seg";
  seg.setAttribute("role", "radiogroup");
  seg.setAttribute("aria-label", "Cleanup");
  for (const [value, label] of CLEANUP_LEVELS) {
    const b = document.createElement("button");
    b.textContent = label;
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String(value === current));
    b.classList.toggle("on", value === current);
    b.onclick = () =>
      updatePreset(p.id, (x) => {
        x.ai = value !== "none";
        if (value !== "none") x.cleanup = value === "medium" ? null : value;
      });
    seg.append(b);
  }
  const hint = CLEANUP_LEVELS.find(([v]) => v === current)[2];
  return presetRow("Cleanup", `${hint}. Instructions and commands always use the AI.`, seg);
}

/** Try the preset on some text (a saved preset; nothing is inserted). */
function tryRow(p) {
  const r = document.createElement("div");
  r.className = "row column";
  const lbl = document.createElement("span");
  lbl.className = "lbl";
  lbl.innerHTML = "Try it<small>Runs this preset on the text, as if you had said it</small>";
  const wrap = document.createElement("div");
  wrap.className = "try";
  const input = document.createElement("input");
  input.type = "text";
  input.spellcheck = false;
  input.value = "um hey so I think we should uh move the call to friday no wait thursday";
  input.setAttribute("aria-label", "Text to try this preset on");
  const out = document.createElement("div");
  out.className = "try-out";
  out.hidden = true;
  const go = button("Try", async () => {
    go.disabled = true;
    out.hidden = false;
    out.textContent = "Working…";
    try {
      out.textContent = await invoke("try_preset", { text: input.value, presetId: p.id });
    } catch (e) {
      out.textContent = String(e);
    }
    go.disabled = false;
  }, "btn primary");
  wrap.append(input, go);
  r.append(lbl, wrap, out);
  return r;
}

// ---------------------------------------------------------------- Extras

function extrasGroup(p) {
  const rows = [];
  rows.push(presetRow("Press Return after", "Sends the message as soon as it's typed. Return while dictating turns it off for that once.", presetSwitch(!!p.auto_send, (on) => updatePreset(p.id, (x) => (x.auto_send = on)))));
  rows.push(presetRow("Tag files for coding agents", "A file name you say, like “chunks dot rs”, becomes an @ mention of that file (for Claude Code, Cursor and others). Uses the project of the terminal's folder or the editor's open file.", presetSwitch(!!p.file_tags, (on) => updatePreset(p.id, (x) => (x.file_tags = on)))));
  rows.push(presetRow("Use the text around the cursor", "Sends up to 1,000 characters already in the field to the AI, so it spells names right and matches the conversation. It may include other people's words.", presetSwitch(!!p.field_context, (on) => updatePreset(p.id, (x) => (x.field_context = on)), !p.ai)));
  rows.push(presetRow("Use what's on the clipboard", "Sends the text you last copied (up to 2,000 characters) to the AI, so it spells names and code right, and so you can say “summarize what I copied”.", presetSwitch(!!p.clipboard_context, (on) => updatePreset(p.id, (x) => (x.clipboard_context = on)), !p.ai)));
  const translate = presetSelect([[null, "Don't translate"], ...TRANSLATE_LANGUAGES.map((l) => [l, l])], p.translate_to, (v) => updatePreset(p.id, (x) => (x.translate_to = v)), "Translate into");
  translate.disabled = !p.ai;
  rows.push(presetRow("Translate into", "Types what you say in another language. Uses the AI.", translate));
  const general = SPOKEN_LANGUAGES.find(([c]) => c === state.settings.general.language)?.[1] ?? "detect automatically";
  rows.push(presetRow("Language you speak", `For this preset only. General is set to ${general}.`, presetSelect([[null, "Same as General"], ...SPOKEN_LANGUAGES], p.language, (v) => updatePreset(p.id, (x) => (x.language = v)), "Language you speak in this preset")));
  rows.push(modelRow(p));
  rows.push(shortcutRow(p));

  const seg = document.createElement("span");
  seg.className = "seg";
  for (const [v, l] of [["paste", "Paste"], ["type", "Type"]]) {
    const b = document.createElement("button");
    b.textContent = l;
    b.classList.toggle("on", p.insert === v);
    b.onclick = () => updatePreset(p.id, (x) => (x.insert = v));
    seg.append(b);
  }
  rows.push(presetRow("Insert by", "Typing is slower; use it where pasting misbehaves", seg));
  return presetGroup("Extras", ...rows);
}

/** The preset's own AI model, from the providers that work right now. */
function modelRow(p) {
  const options = state.preset_ai_options ?? [];
  const current = p.model?.provider ?? null;
  const known = options.some((o) => o.value === current);
  const providers = [[null, "Same as AI Models"], ...options.map((o) => [o.value, o.label])];
  if (current && !known) providers.push([current, `${current} (not available now)`]);
  const sel = presetSelect(providers, current, (v) => updatePreset(p.id, (x) => (x.model = v ? { provider: v, model: null } : null)), "AI model for this preset");
  sel.disabled = !p.ai;
  const wrap = document.createElement("span");
  wrap.className = "model-pick";
  wrap.append(sel);
  const cloudModel = ["openai", "openrouter", "anthropic", "custom"].includes(current);
  if (cloudModel) {
    const name = document.createElement("input");
    name.type = "text";
    name.spellcheck = false;
    name.placeholder = current === "custom" ? "Model name" : `Default (${state.defaults[`${current}_ai`] ?? "provider's choice"})`;
    name.value = p.model?.model ?? "";
    name.setAttribute("aria-label", "Model name");
    name.onchange = () => updatePreset(p.id, (x) => (x.model = { provider: current, model: name.value.trim() || null }));
    wrap.append(name);
  }
  return presetRow("AI model", "For example the local model for chat and a cloud one for email. Local-only mode and spending limits still apply.", wrap);
}

// ---------------------------------------------------------------- Shortcut

let shortcutCapture = null;

function shortcutRow(p) {
  const wrap = document.createElement("span");
  wrap.className = "keypicker shortcut-pick";
  const main = document.createElement("button");
  main.type = "button";
  main.className = "kp-main";
  const cap = document.createElement("span");
  if (p.shortcut) {
    cap.className = "kbd";
    cap.textContent = shortcutLabel(p.shortcut, state.platform);
  } else {
    cap.className = "kp-hint none";
    cap.textContent = "None";
  }
  const hint = document.createElement("span");
  hint.className = "kp-hint";
  hint.textContent = "Click to record";
  main.append(cap, hint);
  main.setAttribute("aria-label", `Shortcut for this preset: ${p.shortcut ? shortcutLabel(p.shortcut, state.platform) : "none"}. Click, then press the shortcut.`);
  main.onclick = () => recordShortcut((s) => updatePreset(p.id, (x) => (x.shortcut = s)), renderPresets, wrap, main);
  wrap.append(main);
  if (p.shortcut) {
    const clear = document.createElement("button");
    clear.type = "button";
    clear.className = "kp-more";
    clear.textContent = "×";
    clear.setAttribute("aria-label", "Remove the shortcut");
    clear.onclick = () => updatePreset(p.id, (x) => (x.shortcut = null));
    wrap.append(clear);
  }
  const r = presetRow("Shortcut", "Starts a hands-free dictation with this preset, whatever app you're in. Press it again to finish.", wrap);
  const error = state.shortcut_errors?.[p.id];
  if (error) {
    const warn = document.createElement("small");
    warn.className = "warn-text";
    warn.textContent = `${shortcutLabel(p.shortcut ?? "", state.platform)} ${error}. Choose another.`;
    r.querySelector(".lbl").append(warn);
  }
  return r;
}

/** Listens for a shortcut on `wrap`'s picker and passes it to `set`;
 *  `redraw` puts the picker back if the user gives up. */
function recordShortcut(set, redraw, wrap, main) {
  stopShortcutCapture();
  invoke("pause_hotkeys", { paused: true }).catch(() => {});
  wrap.classList.add("recording");
  main.setAttribute("aria-pressed", "true");
  main.innerHTML = `<span class="kp-dot" aria-hidden="true"></span><span>Press a shortcut…</span><span class="kp-hint">Esc cancels</span>`;
  const onKey = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "Escape") return stopShortcutCapture(true);
    if (/^(Shift|Control|Alt|Meta)(Left|Right)$/.test(e.code) || e.code === "Fn") return;
    const mods = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push(state.platform === "macos" ? "Cmd" : "Super");
    const plainKey = !/^F\d+$/.test(e.code);
    if (plainKey && (!mods.length || (mods.length === 1 && mods[0] === "Shift"))) {
      main.querySelector("span:nth-child(2)").textContent = state.platform === "macos" ? "Add ⌃, ⌥ or ⌘" : "Add Ctrl, Alt or Win";
      return;
    }
    stopShortcutCapture();
    set([...mods, e.code].join("+"));
  };
  window.addEventListener("keydown", onKey, true);
  shortcutCapture = { onKey, redraw, timer: setTimeout(() => stopShortcutCapture(true), 14000) };
}

function stopShortcutCapture(redraw = false) {
  if (!shortcutCapture) return;
  window.removeEventListener("keydown", shortcutCapture.onKey, true);
  clearTimeout(shortcutCapture.timer);
  const again = shortcutCapture.redraw;
  shortcutCapture = null;
  invoke("pause_hotkeys", { paused: false }).catch(() => {});
  if (redraw) again();
}
window.addEventListener("blur", () => stopShortcutCapture(true));

// ---------------------------------------------------------------- By app

function renderAppsView() {
  const list = $("apps-list");
  const rows = [];
  const seen = new Set();
  for (const a of recentApps) {
    for (const entry of [{ id: a.id, name: a.name }, ...(a.site ? [{ id: a.id, name: a.name, site: a.site }] : [])]) {
      const key = entry.site ?? entry.id;
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push(appRow(entry));
    }
  }
  if (!rows.length) {
    const empty = document.createElement("div");
    empty.className = "row";
    empty.innerHTML = '<span class="lbl">Nothing yet<small>Apps and websites you dictate into appear here.</small></span>';
    rows.push(empty);
  }
  list.replaceChildren(...rows);
}

function appRow(a) {
  const ctx = { app_id: a.id, app_name: a.name, url: a.site ?? null, title: null };
  const current = matchPreset(state.presets, ctx);
  const r = document.createElement("div");
  r.className = "row app-row";
  const icon = document.createElement("span");
  icon.className = "chip-icon big";
  icon.setAttribute("aria-hidden", "true");
  if (a.site) icon.textContent = "🌐";
  else appIcon(icon, a.id, appName(a.id));
  const lbl = document.createElement("span");
  lbl.className = "lbl";
  lbl.textContent = a.site ?? appName(a.id);
  if (a.site) {
    const s = document.createElement("small");
    s.textContent = `in ${appName(a.id)}`;
    lbl.append(s);
  }
  const sel = presetSelect(state.presets.map((p) => [p.id, p.name]), current?.id, (v) => assignApp(a, v ?? "default"), `Preset for ${a.site ?? appName(a.id)}`);
  r.append(icon, lbl, sel);
  return r;
}

/** Gives an app (or website) to one preset: drops its own rules from the
 *  others and adds one to the chosen preset (none needed for Default). */
function assignApp(a, presetId) {
  const presets = structuredClone(state.presets);
  for (const p of presets) {
    p.rules = p.rules.filter((r) => !(a.site ? r.url === a.site && !r.app && !r.title : r.app === a.id && !r.url && !r.title));
  }
  const target = presets.find((p) => p.id === presetId);
  if (target && target.id !== "default") target.rules.push(a.site ? { app: null, url: a.site, title: null } : { app: a.id, url: null, title: null });
  const now = matchPreset(presets, { app_id: a.id, app_name: a.name, url: a.site ?? null, title: null });
  if (now?.id !== presetId) {
    toast(`“${now?.name}” still matches ${a.site ?? appName(a.id)} through a broader rule. Edit that preset to change it.`, true);
  }
  savePresets(presets);
}

// ---------------------------------------------------------------- changes

function movePreset(id, to) {
  const presets = structuredClone(state.presets);
  const from = presets.findIndex((p) => p.id === id);
  if (from < 0 || to < 0 || to >= presets.length || from === to) return;
  const [moved] = presets.splice(from, 1);
  presets.splice(to, 0, moved);
  savePresets(presets);
}

/** "New preset": pick a starting point. */
async function chooseTemplate() {
  const dialog = document.createElement("dialog");
  dialog.className = "confirm templates";
  const title = document.createElement("p");
  title.className = "confirm-title";
  title.textContent = "New preset";
  const grid = document.createElement("div");
  grid.className = "template-grid";
  for (const t of TEMPLATES) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "template";
    const name = document.createElement("b");
    name.textContent = t.name;
    const about = document.createElement("span");
    about.textContent = t.about;
    b.append(name, about);
    b.onclick = () => {
      dialog.close();
      addPreset(t);
    };
    grid.append(b);
  }
  const acts = document.createElement("div");
  acts.className = "confirm-acts";
  const file = document.createElement("input");
  file.type = "file";
  file.accept = ".toml";
  file.hidden = true;
  file.onchange = async () => {
    const chosen = file.files[0];
    if (!chosen) return;
    dialog.close();
    try {
      importPreset(await invoke("read_preset_file", { text: await chosen.text() }));
    } catch (e) {
      toast(String(e), true);
    }
  };
  acts.append(file, button("Import a preset file…", () => file.click(), "link"), button("Cancel", () => dialog.close()));
  dialog.append(title, grid, acts);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  grid.querySelector("button")?.focus();
}

/** Adds a preset from a shared file, renamed if the name is taken. */
function importPreset(p) {
  const id = `custom-${Date.now().toString(36)}`;
  let name = p.name || "Imported preset";
  for (let n = 2; state.presets.some((x) => x.name === name); n++) name = `${p.name} ${n}`;
  selectedPreset = id;
  savePresets([...state.presets, { ...p, id, name, shortcut: null }]);
  toast(`Added the “${name}” preset`);
}

function addPreset(t) {
  const id = `custom-${Date.now().toString(36)}`;
  selectedPreset = id;
  savePresets([
    ...state.presets,
    {
      id,
      name: t.name === "Blank" ? "New preset" : t.name,
      instructions: t.instructions ?? "",
      ai: true,
      insert: "paste",
      rules: [],
      field_context: false,
      clipboard_context: !!t.clipboard_context,
      translate_to: t.translate_to ?? null,
      tone: t.tone ?? null,
      cleanup: null,
      file_tags: !!t.file_tags,
      auto_send: !!t.auto_send,
      language: null,
      model: null,
      shortcut: null,
    },
  ]);
}

function updatePreset(id, change) {
  const presets = structuredClone(state.presets);
  const p = presets.find((x) => x.id === id);
  if (!p) return;
  change(p);
  // Rules still being typed are kept (empty value), not saved as matches.
  savePresets(presets);
}

function savePresets(presets) {
  const s = collect();
  s.presets = presets;
  state.presets = presets;
  renderPresets();
  save(s);
}

document.querySelectorAll(".view-switch [data-view]").forEach((b) =>
  b.addEventListener("click", () => {
    presetView = b.dataset.view;
    renderPresets();
  })
);

$("presets-reset").addEventListener("click", async () => {
  const sure = await askConfirm("Reset all presets to the built-in ones?", {
    action: "Reset",
    detail: "Your changes to presets, and any presets you added, will be lost.",
  });
  if (!sure) return;
  selectedPreset = "default";
  const s = collect();
  s.presets = null;
  save(s);
});
