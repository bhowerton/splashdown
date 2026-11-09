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
  renderHooks(state.settings.hooks ?? []);
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

// ---------------------------------------------------------------- hooks

/** When a hook runs, as the list and the editor say it. */
const HOOK_EVENTS = [
  ["dictation", "After a dictation"],
  ["before-insert", "Before a dictation goes in (changes its text)"],
  ["meeting", "When a meeting ends"],
  ["transcript", "When a transcript is finished"],
  ["phone-note", "When a note comes from your iPhone"],
];

/** A preset's name, from its id. */
function presetName(id) {
  return (state.presets ?? []).find((p) => p.id === id)?.name ?? id;
}

/** What a hook runs, in a few words. */
function hookSummary(h) {
  if (h.kind === "web") return `Sends to ${h.url}`;
  return `Runs ${h.path}${h.args?.length ? ` ${h.args.join(" ")}` : ""}`;
}

/** What a hook does, in a sentence, for the editor. */
function hookSentence(h) {
  const what = h.kind === "web"
    ? `Splashdown sends the text and its details to ${h.url || "the web address"}, signed with your webhook secret.`
    : `Splashdown runs ${h.path || "the program"} with the text as its input, never through a shell.`;
  const preset = h.preset ? ` with the ${presetName(h.preset)} preset` : "";
  const when = {
    dictation: ` After every dictation${preset} that goes into a text field.`,
    "before-insert": ` Before each dictation${preset} goes in: what it writes back is inserted instead. If it takes over 2 seconds or fails, the dictation goes in as said.`,
    meeting: " When a meeting's note is finished, with its summary.",
    transcript: " When a transcript of a file, a link, an app's audio or the microphone is finished.",
    "phone-note": " For each note from your iPhone.",
  }[h.when];
  return what + when;
}

/** The hooks, each with Turn off or on, Edit and Delete. */
function renderHooks(hooks) {
  $("hk-secret-row").hidden = !hooks.some((h) => h.kind === "web");
  $("hk-list").replaceChildren(
    ...hooks.map((h, i) => {
      const on = h.on !== false;
      const row = document.createElement("div");
      row.className = "row";
      row.classList.toggle("dim", !on);
      const lbl = document.createElement("span");
      lbl.className = "lbl";
      const when = HOOK_EVENTS.find(([v]) => v === h.when)?.[1] ?? h.when;
      lbl.textContent = h.preset ? `${when} · ${presetName(h.preset)}` : when;
      const small = document.createElement("small");
      small.textContent = hookSummary(h) + (on ? "" : " · off");
      lbl.append(small);
      const buttons = document.createElement("span");
      buttons.className = "ma-buttons";
      buttons.append(
        button(on ? "Turn off" : "Turn on", () => updateHooks((list) => (list[i] = { ...list[i], on: !on })), "link"),
        button("Edit…", () => editHook(i), "btn"),
        button("Delete", async () => {
          if (await askConfirm("Delete this hook?", { detail: hookSummary(h) })) updateHooks((list) => list.splice(i, 1));
        }, "btn"),
      );
      row.append(lbl, buttons);
      return row;
    }),
  );
}

/** Changes the list of hooks and saves. */
function updateHooks(change) {
  const s = collect();
  s.hooks = [...(state.settings.hooks ?? [])];
  change(s.hooks);
  save(s);
}

$("hk-add").addEventListener("click", () => editHook(null));
$("hk-docs").addEventListener("click", () => invoke("hooks_docs_page").catch((e) => toast(String(e), true)));
$("hk-secret-copy").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(await invoke("hooks_secret"));
    toast("Webhook secret copied");
  } catch (e) {
    toast(String(e), true);
  }
});
$("hk-secret-new").addEventListener("click", async () => {
  const sure = await askConfirm("Make a new webhook secret?", {
    action: "Make a new one",
    detail: "Whatever checks the old one stops accepting Splashdown's webhooks until you give it the new one.",
  });
  if (!sure) return;
  try {
    await navigator.clipboard.writeText(await invoke("hooks_new_secret"));
    toast("New webhook secret copied");
  } catch (e) {
    toast(String(e), true);
  }
});

/** The editor for a new hook (`index` null) or an existing one. */
function editHook(index) {
  const hooks = state.settings.hooks ?? [];
  const h = index === null ? { when: "meeting", kind: "web", url: "" } : hooks[index];
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
  title.textContent = index === null ? "Add a hook" : "Edit hook";
  const when = document.createElement("select");
  for (const [value, label] of HOOK_EVENTS) when.append(new Option(label, value));
  when.value = h.when;
  const preset = document.createElement("select");
  const presetPart = field("Preset", preset);
  const kind = document.createElement("select");
  kind.append(new Option("Run a script or program", "program"), new Option("Send to a web address", "web"));
  kind.value = h.kind;
  const url = text(h.url, "https://hooks.zapier.com/hooks/catch/…");
  const path = text(h.path, mac ? "/Users/you/bin/to-slack.sh" : "C:\\Scripts\\to-slack.ps1");
  const pick = button("Choose…", async () => {
    const picked = await invoke("my_action_pick_program").catch(() => null);
    if (picked) {
      path.value = picked;
      describe();
    }
  }, "btn");
  pick.type = "button";
  const pathRow = document.createElement("div");
  pathRow.className = "ma-pick";
  pathRow.append(path, pick);
  const args = document.createElement("textarea");
  args.value = (h.args ?? []).join("\n");
  args.placeholder = "--channel\n#notes";
  args.spellcheck = false;
  const webPart = field("Web address", url, "The text and its details are sent as JSON.");
  const programPart = document.createElement("div");
  programPart.append(
    field("Script or program", pathRow, "It gets the text as its input, and the details in SPLASHDOWN_ variables."),
    field("Arguments", args, "One per line, never through a shell."),
  );
  const sentence = document.createElement("p");
  sentence.className = "ma-note";
  const result = document.createElement("p");
  result.className = "ma-note";
  result.hidden = true;
  const error = document.createElement("p");
  error.className = "ma-error";
  error.hidden = true;
  // The hook as the form has it now.
  const current = () => {
    const hook = { when: when.value, kind: kind.value, on: h.on !== false };
    if (preset.value) hook.preset = preset.value;
    if (kind.value === "web") hook.url = url.value.trim();
    else {
      hook.path = path.value.trim();
      hook.args = args.value.split("\n").map((a) => a.trim()).filter(Boolean);
    }
    return hook;
  };
  const describe = () => {
    sentence.textContent = hookSentence(current());
  };
  const showPresets = () => {
    const changes = when.value === "before-insert";
    const kept = preset.value || h.preset || "";
    preset.replaceChildren(
      ...(changes ? [] : [new Option("Every preset", "")]),
      ...(state.presets ?? []).map((p) => new Option(p.name, p.id)),
    );
    preset.value = [...preset.options].some((o) => o.value === kept) ? kept : preset.options[0]?.value ?? "";
    presetPart.hidden = when.value !== "dictation" && !changes;
    if (!presetPart.hidden) return;
    preset.replaceChildren(new Option("", ""));
    preset.value = "";
  };
  const showKind = () => {
    // Changing the text: only a program, which can answer in time.
    const changes = when.value === "before-insert";
    kind.options[1].disabled = changes;
    if (changes) kind.value = "program";
    webPart.hidden = kind.value !== "web";
    programPart.hidden = kind.value !== "program";
  };
  const refresh = () => {
    showPresets();
    showKind();
    describe();
    result.hidden = true;
  };
  when.addEventListener("change", refresh);
  kind.addEventListener("change", refresh);
  preset.addEventListener("change", describe);
  for (const input of [url, path, args]) input.addEventListener("input", describe);
  refresh();
  const check = async (hook) => {
    const problem = await invoke("hooks_problem", { hook }).catch((err) => String(err));
    error.textContent = problem ?? "";
    error.hidden = !problem;
    return !problem;
  };
  const acts = document.createElement("div");
  acts.className = "confirm-acts";
  const test = button("Test", async () => {
    const hook = current();
    if (!(await check(hook))) return;
    test.disabled = true;
    result.hidden = false;
    result.classList.remove("warn-text");
    result.textContent = "Testing…";
    try {
      result.textContent = await invoke("hooks_test", { hook });
    } catch (e) {
      result.textContent = String(e);
      result.classList.add("warn-text");
    } finally {
      test.disabled = false;
    }
  }, "btn");
  test.type = "button";
  const cancel = button("Cancel", () => dialog.close(), "btn");
  cancel.type = "button";
  const ok = document.createElement("button");
  ok.className = "btn primary";
  ok.textContent = "Save";
  acts.append(test, cancel, ok);
  form.append(
    title,
    field("When", when),
    presetPart,
    field("What it does", kind),
    webPart,
    programPart,
    sentence,
    result,
    error,
    acts,
  );
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const hook = current();
    if (!(await check(hook))) return;
    // Saved here, with what it does said above: it runs without asking.
    hook.trusted = true;
    dialog.close();
    updateHooks((list) => (index === null ? list.push(hook) : (list[index] = hook)));
  });
  dialog.append(form);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  when.focus();
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
