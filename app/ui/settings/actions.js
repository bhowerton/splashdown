"use strict";

// Actions tab: whether spoken commands may do things on this computer
// (voice actions), which ones, the choices that go with them, and the
// user's own actions (a link, a script or program, keys to press).

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
  renderMine(a.mine ?? [], !!a.enabled);
}

// ---------------------------------------------------------------- your actions

/** What one of the user's actions does, in a few words. */
function mineSummary(m) {
  if (m.kind === "link") return `Opens ${m.url}`;
  if (m.kind === "program") return `Runs ${m.path}${m.args?.length ? ` ${m.args.join(" ")}` : ""}`;
  return `Presses ${m.keys}`;
}

/** The user's actions, each with Edit and Delete. */
function renderMine(mine, enabled) {
  $("ac-mine").classList.toggle("dim", !enabled);
  $("ac-mine-add").disabled = !enabled;
  $("ac-mine-list").replaceChildren(
    ...mine.map((m, i) => {
      const row = document.createElement("div");
      row.className = "row";
      const lbl = document.createElement("span");
      lbl.className = "lbl";
      lbl.textContent = m.name;
      const small = document.createElement("small");
      small.textContent = mineSummary(m) + (m.trusted ? " · runs without asking" : "");
      lbl.append(small);
      const buttons = document.createElement("span");
      buttons.className = "ma-buttons";
      if (m.trusted) {
        buttons.append(button("Ask again", () => updateMine((list) => (list[i] = { ...list[i], trusted: false })), "link"));
      }
      const edit = button("Edit…", () => editMine(i), "btn");
      edit.disabled = !enabled;
      const del = button("Delete", async () => {
        if (await askConfirm(`Delete “${m.name}”?`)) updateMine((list) => list.splice(i, 1));
      }, "btn");
      del.disabled = !enabled;
      buttons.append(edit, del);
      row.append(lbl, buttons);
      return row;
    }),
  );
}

/** Changes the list of actions and saves. */
function updateMine(change) {
  const s = collect();
  s.actions.mine = [...(state.settings.actions?.mine ?? [])];
  change(s.actions.mine);
  save(s);
}

$("ac-mine-add").addEventListener("click", () => editMine(null));

/** The editor for a new action (`index` null) or an existing one. */
function editMine(index) {
  const mine = state.settings.actions?.mine ?? [];
  const m = index === null ? { name: "", kind: "link", url: "" } : mine[index];
  const mac = state.platform === "macos";
  const dialog = document.createElement("dialog");
  dialog.className = "confirm my-action";
  const form = document.createElement("form");
  form.className = "ma-form";
  const field = (label, input, note) => {
    const l = document.createElement("label");
    l.textContent = label;
    if (note) {
      const small = document.createElement("small");
      small.textContent = ` ${note}`;
      l.append(small);
    }
    const wrap = document.createElement("div");
    wrap.append(l, input);
    return wrap;
  };
  const text = (value, placeholder) => {
    const input = document.createElement("input");
    input.type = "text";
    input.value = value ?? "";
    input.placeholder = placeholder;
    input.spellcheck = false;
    return input;
  };
  const title = document.createElement("p");
  title.className = "confirm-title";
  title.textContent = index === null ? "Add an action" : `Edit “${m.name}”`;
  const name = text(m.name, "Deploy staging");
  const kind = document.createElement("select");
  for (const [value, label] of [
    ["link", "Open a link"],
    ["program", "Run a script or program"],
    ["keys", "Press keys"],
  ]) {
    kind.append(new Option(label, value));
  }
  kind.value = m.kind;
  const url = text(m.url, "https://linear.app/acme/issue/{text}");
  const path = text(m.path, mac ? "/Users/you/bin/deploy.sh" : "C:\\Scripts\\deploy.ps1");
  const pick = button("Choose…", async () => {
    const picked = await invoke("my_action_pick_program").catch(() => null);
    if (picked) path.value = picked;
  }, "btn");
  pick.type = "button";
  const pathRow = document.createElement("div");
  pathRow.className = "ma-pick";
  pathRow.append(path, pick);
  const args = document.createElement("textarea");
  args.value = (m.args ?? []).join("\n");
  args.placeholder = "--site\n{text}";
  args.spellcheck = false;
  const keys = text(m.keys, mac ? "cmd+shift+4" : "ctrl+shift+n");
  const about = text(m.about, "Optional: what it does, so it's picked when you say it differently");
  const linkPart = field("Link", url, "Put {text} where what you say goes.");
  const programPart = document.createElement("div");
  programPart.append(
    field("Script or program", pathRow),
    field("Arguments", args, "One per line, never through a shell. {text} is what you say."),
  );
  const keysPart = field("Keys", keys, mac ? "Such as cmd+shift+4, right or f5." : "Such as ctrl+shift+n, right or f5.");
  const showKind = () => {
    linkPart.hidden = kind.value !== "link";
    programPart.hidden = kind.value !== "program";
    keysPart.hidden = kind.value !== "keys";
  };
  kind.addEventListener("change", showKind);
  showKind();
  const error = document.createElement("p");
  error.className = "ma-error";
  error.hidden = true;
  const acts = document.createElement("div");
  acts.className = "confirm-acts";
  const cancel = button("Cancel", () => dialog.close(), "btn");
  cancel.type = "button";
  const ok = document.createElement("button");
  ok.className = "btn primary";
  ok.textContent = "Save";
  acts.append(cancel, ok);
  form.append(
    title,
    field("Name", name, "What you'll say to run it."),
    field("What it does", kind),
    linkPart,
    programPart,
    keysPart,
    field("Description", about),
    error,
    acts,
  );
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const action = { name: name.value.trim(), kind: kind.value, trusted: !!m.trusted };
    if (about.value.trim()) action.about = about.value.trim();
    if (kind.value === "link") action.url = url.value.trim();
    if (kind.value === "program") {
      action.path = path.value.trim();
      action.args = args.value.split("\n").map((a) => a.trim()).filter(Boolean);
    }
    if (kind.value === "keys") action.keys = keys.value.trim().toLowerCase();
    // What it runs changed: ask again before the first run.
    if (index !== null && mineSummary(action) !== mineSummary(m)) action.trusted = false;
    const taken = mine.some((x, i) => i !== index && x.name.toLowerCase() === action.name.toLowerCase());
    const problem = taken
      ? "Another action has that name."
      : await invoke("my_action_problem", { action }).catch((err) => String(err));
    if (problem) {
      error.textContent = problem;
      error.hidden = false;
      return;
    }
    dialog.close();
    updateMine((list) => (index === null ? list.push(action) : (list[index] = action)));
  });
  dialog.append(form);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  name.focus();
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
