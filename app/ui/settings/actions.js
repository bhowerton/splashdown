"use strict";

// Actions tab: whether spoken commands may do things on this computer
// (voice actions), which ones, and the choices that go with them.

function renderActions() {
  const a = state.settings.actions ?? {};
  const mac = state.platform === "macos";
  const instruction = state.settings.hotkeys.instruction ?? defaultKey("hk-instruction");
  const key = state.key_options.find((k) => k.value === instruction);
  document.querySelectorAll(".instr-key").forEach((el) => (el.textContent = key ? key.label : "the instruction key"));
  $("ac-enabled").checked = !!a.enabled;
  const off = new Set(a.off ?? []);
  document.querySelectorAll("[data-action]").forEach((el) => {
    el.checked = !off.has(el.dataset.action);
    el.disabled = !a.enabled;
  });
  $("ac-list").classList.toggle("dim", !a.enabled);
  $("ac-choices").hidden = !a.enabled;
  $("ac-shortcut-row").hidden = !mac;
  $("ac-event-note").textContent = mac
    ? "To your calendar. macOS asks you to allow it the first time."
    : "Opened in your calendar app (such as Outlook) for you to save.";
  $("ac-reminder-note").textContent = mac
    ? "In Reminders, with an alert at the time you said."
    : "Added to today's note as a to-do with its time: Windows has no reminders app to add to.";
  setValue($("ac-engine"), a.search_engine ?? "google");
  $("ac-confirm").checked = !!a.confirm_events;
  // On Windows the calendar app shows each event before it's saved.
  $("ac-confirm").closest(".row").hidden = !mac;
  renderTrusted(a.trusted_shortcuts ?? [], mac);
}

/** Shortcuts allowed with "Always allow", each with a way to take it back. */
function renderTrusted(names, mac) {
  $("ac-trusted-row").hidden = !mac;
  const el = $("ac-trusted");
  if (!names.length) {
    el.textContent = "None yet. Choose Always allow when Splashdown asks before running one.";
    return;
  }
  el.replaceChildren(
    ...names.map((name) => {
      const chip = document.createElement("span");
      chip.className = "chip";
      chip.textContent = name;
      const remove = document.createElement("button");
      remove.className = "link";
      remove.textContent = "Ask again";
      remove.setAttribute("aria-label", `Ask before running ${name} again`);
      remove.addEventListener("click", () => {
        const s = collect();
        s.actions.trusted_shortcuts = names.filter((n) => n !== name);
        save(s);
      });
      chip.append(" ", remove);
      return chip;
    }),
  );
}

/** The Actions tab's part of the settings, for collect(). */
function collectActions(previous) {
  const a = { ...(previous ?? {}) };
  a.enabled = $("ac-enabled").checked;
  // Actions turned off; ones this platform doesn't show keep their state.
  const shown = [...document.querySelectorAll("[data-action]")].filter((el) => !el.closest("[hidden]"));
  const hiddenOff = (a.off ?? []).filter((id) => !shown.some((el) => el.dataset.action === id));
  a.off = [...hiddenOff, ...shown.filter((el) => !el.checked).map((el) => el.dataset.action)];
  a.search_engine = $("ac-engine").value || "google";
  a.confirm_events = $("ac-confirm").checked;
  a.trusted_shortcuts = a.trusted_shortcuts ?? [];
  return a;
}
