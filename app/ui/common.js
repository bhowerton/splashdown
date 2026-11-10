"use strict";

// Helpers shared by every window in app/ui (Settings, the setup guide,
// Notes, Ask, Compose and the small ones). Loaded first.
// The pages use classic scripts (not modules, so they also open as file://),
// so every top-level name is shared across a page's scripts: keep them unique.

// Talks to the Rust side through Tauri's global API. Opened as a plain file
// in a browser (for design work), calls go to the page's mockInvoke(cmd, args),
// which must be loaded before anything calls invoke.
const tauri = window.__TAURI__;
const invoke = tauri
  ? tauri.core.invoke
  : async (cmd, args) => {
      // Pages without a sample backend still open; their calls just fail.
      if (typeof mockInvoke !== "function") throw new Error(`no backend for ${cmd} in a plain browser page`);
      return mockInvoke(cmd, args);
    };
const listen = tauri ? tauri.event.listen : mockListen;

/** Events the page's mock sends (`mockEmit`), when opened as a plain file. */
const mockListeners = new Map();

async function mockListen(name, handler) {
  if (!mockListeners.has(name)) mockListeners.set(name, new Set());
  mockListeners.get(name).add(handler);
  return () => mockListeners.get(name).delete(handler);
}

function mockEmit(name, payload) {
  for (const handler of mockListeners.get(name) ?? []) handler({ payload });
}

const $ = (id) => document.getElementById(id);

function badge(text, kind) {
  const b = document.createElement("span");
  b.className = `badge ${kind}`;
  b.textContent = text;
  return b;
}

function button(text, onclick, cls = "btn") {
  const b = document.createElement("button");
  b.className = cls;
  b.textContent = text;
  b.onclick = onclick;
  return b;
}

/**
 * Asks before something that can't be undone. The webview has no working
 * `window.confirm` (it returns false without showing anything), so this draws
 * its own modal. Resolves true only if the action button is clicked.
 */
function askConfirm(message, { action = "Delete", detail = "", danger = true } = {}) {
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "confirm";
    const title = document.createElement("p");
    title.className = "confirm-title";
    title.textContent = message;
    dialog.append(title);
    if (detail) {
      const small = document.createElement("p");
      small.className = "confirm-detail";
      small.textContent = detail;
      dialog.append(small);
    }
    const acts = document.createElement("div");
    acts.className = "confirm-acts";
    const cancel = button("Cancel", () => dialog.close("cancel"));
    const ok = button(action, () => dialog.close("ok"), danger ? "btn danger-solid" : "btn primary");
    acts.append(cancel, ok);
    dialog.append(acts);
    dialog.addEventListener("close", () => {
      resolve(dialog.returnValue === "ok");
      dialog.remove();
    });
    document.body.append(dialog);
    dialog.showModal();
    // The safe choice has focus, so Return doesn't delete by accident.
    cancel.focus();
  });
}

/** Asks for a line of text in a small modal; resolves to it, or null. */
function askText(message, { detail = "", placeholder = "", action = "OK", value = "" } = {}) {
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "confirm";
    const form = document.createElement("form");
    form.method = "dialog";
    const title = document.createElement("p");
    title.className = "confirm-title";
    title.textContent = message;
    form.append(title);
    if (detail) {
      const small = document.createElement("p");
      small.className = "confirm-detail";
      small.textContent = detail;
      form.append(small);
    }
    const input = document.createElement("input");
    input.type = "text";
    input.placeholder = placeholder;
    input.value = value;
    input.spellcheck = false;
    input.style.width = "100%";
    input.style.boxSizing = "border-box";
    const acts = document.createElement("div");
    acts.className = "confirm-acts";
    const cancel = button("Cancel", () => dialog.close("cancel"));
    cancel.type = "button";
    const ok = document.createElement("button");
    ok.className = "btn primary";
    ok.textContent = action;
    ok.value = "ok";
    acts.append(cancel, ok);
    form.append(input, acts);
    dialog.append(form);
    dialog.addEventListener("close", () => {
      resolve(dialog.returnValue === "ok" && input.value.trim() ? input.value.trim() : null);
      dialog.remove();
    });
    document.body.append(dialog);
    dialog.showModal();
    input.focus();
    input.select();
  });
}

let toastTimer;
/** Calls a command for its effect, showing what went wrong if it fails
 *  (a toast where the page has one, the console otherwise). Use `invoke`
 *  directly when the caller handles the error itself. */
async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    toast(`${e}`, true, cmd);
  }
}

/** Shows a short message, or writes it to the console on pages without a
 *  #toast (the offer, the pill, the prompt). `source` prefixes the console
 *  line, e.g. the command that failed. */
function toast(text, error = false, source = "") {
  const t = $("toast");
  if (!t) {
    const line = source ? `${source}: ${text}` : text;
    if (error) console.error(line);
    else console.log(line);
    return;
  }
  t.textContent = text;
  t.className = `toast${error ? " error" : ""}`;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), error ? 6000 : 1800);
}

// ---------------------------------------------------------------- text

/** "1 note", "3 notes"; `many` for words that don't just add an s. */
function plural(n, one, many = `${one}s`) {
  return `${n} ${n === 1 ? one : many}`;
}

/** A length of time as a clock shows it: "12:30", or "1:02:30" past an
 *  hour. */
function clock(seconds) {
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const rest = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${rest}` : `${m}:${rest}`;
}

/** Mac symbols for shortcut modifiers. */
const MOD_SYMBOL = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Cmd: "⌘", Super: "⊞" };

/** A saved shortcut as this computer writes it: "Ctrl+Alt+KeyE" → "⌃⌥E"
 *  on a Mac (`platform` "macos"), "Ctrl+Alt+E" elsewhere. */
function shortcutLabel(s, platform) {
  const parts = s.split("+");
  const key = parts.pop().replace(/^Key/, "").replace(/^Digit/, "");
  if (platform !== "macos") return [...parts, key].join("+");
  return parts.map((m) => MOD_SYMBOL[m] ?? m).join("") + key;
}

// ---------------------------------------------------------------- dictation key

/** A ⌃⌥ shortcut as this computer writes it: "⌃⌥N" on a Mac, "Ctrl+Alt+N"
 *  elsewhere. */
function comboLabel(letter, platform) {
  return (platform === "macos" ? "⌃⌥" : "Ctrl+Alt+") + letter;
}

/** Fills in what differs by platform in static text: `data-combo="N"`
 *  elements get the shortcut (comboLabel), `.menubar` elements "the menu
 *  bar" on a Mac and "Splashdown's icon by the clock" on Windows, and
 *  `.alt-key` elements "Option" or "Alt". */
function fitPlatform(platform, root = document) {
  const mac = platform === "macos";
  root.querySelectorAll("[data-combo]").forEach((el) => (el.textContent = comboLabel(el.dataset.combo, platform)));
  root.querySelectorAll(".menubar").forEach((el) => (el.textContent = mac ? "the menu bar" : "Splashdown's icon by the clock"));
  root.querySelectorAll(".alt-key").forEach((el) => (el.textContent = mac ? "Option" : "Alt"));
}

/** The dictation key used while none is chosen. */
function defaultDictationKey(platform) {
  return platform === "macos" ? "right-alt" : "right-control";
}

/** The chosen dictation key's label, e.g. "Right Option". */
function dictationKeyLabel(snapshot) {
  const value = snapshot.settings.hotkeys.dictation ?? defaultDictationKey(snapshot.platform);
  const key = snapshot.key_options.find((k) => k.value === value);
  return key ? key.label : "the dictation key";
}

/** The chosen instruction key's label, e.g. "Right Shift". */
function instructionKeyLabel(snapshot) {
  const value = snapshot.settings.hotkeys.instruction ?? "right-shift";
  const key = snapshot.key_options.find((k) => k.value === value);
  return key ? key.label : "the instruction key";
}

/** Fills every `.dict-key` placeholder in the page. */
function showDictationKey(snapshot) {
  const label = dictationKeyLabel(snapshot);
  document.querySelectorAll(".dict-key").forEach((el) => (el.textContent = label));
}

// ---------------------------------------------------------------- permissions

function openPerm(permission) {
  call("open_permission", { permission });
}

/** The Accessibility and Microphone rows. Both pages use the same markup:
 *  #<prefix>ax-step / -state / -howto and #<prefix>mic-step / -state / -howto,
 *  with `dotClass` on the numbered step marker. */
function renderPermissionRows(p, prefix, dotClass) {
  $(`${prefix}ax-step`).className = `${dotClass}${p.accessibility ? " done" : ""}`;
  const ax = $(`${prefix}ax-state`);
  if (p.accessibility) {
    ax.replaceChildren(badge("Allowed", "ok"));
    $(`${prefix}ax-howto`).hidden = true;
  } else if (!ax.querySelector("button")) {
    ax.replaceChildren(
      button("Allow…", () => {
        call("request_accessibility");
        $(`${prefix}ax-howto`).hidden = false;
        // The system prompt only appears once; offer a direct route too.
        if (!ax.querySelector(".open")) {
          ax.prepend(button("Open System Settings", () => openPerm("accessibility"), "btn open"));
        }
      }, "btn primary")
    );
  }

  $(`${prefix}mic-step`).className = `${dotClass}${p.microphone === "granted" ? " done" : ""}`;
  const mic = $(`${prefix}mic-state`);
  const windows = state?.platform === "windows";
  $(`${prefix}mic-howto`).hidden = p.microphone !== "denied";
  if (windows) {
    $(`${prefix}mic-howto`).innerHTML =
      "Microphone access is off for desktop apps. In Settings → Privacy &amp; security → Microphone, turn on <b>Let desktop apps access your microphone</b>.";
  }
  if (p.microphone === "granted") {
    mic.replaceChildren(badge("Allowed", "ok"));
  } else if (p.microphone === "denied") {
    const label = windows ? "Open Settings" : "Open System Settings";
    mic.replaceChildren(button(label, () => openPerm("microphone"), "btn primary"));
  } else if (!mic.querySelector("button")) {
    mic.replaceChildren(
      button("Allow…", async (e) => {
        e.target.disabled = true;
        try {
          await invoke("request_microphone");
        } catch (err) {
          toast(String(err), true);
        }
        e.target.disabled = false;
      }, "btn primary")
    );
  }
}

/** Whether the user is typing in a field inside `host`. Lists don't redraw
 *  then, but a focused button (Chromium focuses one on click) doesn't count:
 *  after a × the list must redraw, or its rows keep their old positions. */
function typingIn(host) {
  const el = document.activeElement;
  return !!el && host.contains(el) && el.matches("input, textarea, select, [contenteditable]");
}

/** Whether the keyboard is in a field of `host` that takes text. For parts
 *  that also hold tick boxes and menus, which must be redrawn after one of
 *  those changes but not while words are being typed. */
function typingTextIn(host) {
  const el = document.activeElement;
  return (
    !!el &&
    host.contains(el) &&
    el.matches('textarea, input:not([type]), input[type="text"], input[type="password"], input[type="search"], input[type="url"], [contenteditable]')
  );
}
