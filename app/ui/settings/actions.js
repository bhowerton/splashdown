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
  loadHookRuns();
  // Which AI a hook's instruction goes to, for the list and the editor.
  invoke("hooks_ai")
    .then((ai) => {
      hookAi = ai;
      renderHooks(state.settings.hooks ?? []);
    })
    .catch(() => {});
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
  ["schedule", "On a schedule, with your notes since"],
  ["before-meeting", "Before a meeting on your calendar"],
];

/** The days a scheduled hook can run on, as the settings name them. */
const HOOK_DAYS = [
  ["mon", "Monday"],
  ["tue", "Tuesday"],
  ["wed", "Wednesday"],
  ["thu", "Thursday"],
  ["fri", "Friday"],
  ["sat", "Saturday"],
  ["sun", "Sunday"],
];
const HOOK_WEEKDAYS = ["mon", "tue", "wed", "thu", "fri"];

/** How far back a scheduled hook's notes go. */
const HOOK_COVERS = [
  ["day", "The last day"],
  ["week", "The last 7 days"],
];

/** Days in words: "Fridays", "Mondays, Wednesdays and Fridays". */
function daysWords(list) {
  const names = HOOK_DAYS.filter(([d]) => list.includes(d)).map(([, name]) => `${name}s`);
  const last = names.pop();
  return names.length ? `${names.join(", ")} and ${last}` : last;
}

/** A schedule's time as this computer writes times: "4:00 PM", "16:00". */
function timeWords(at) {
  const [hour, minute] = (at ?? "").split(":").map(Number);
  if (!Number.isInteger(hour) || !Number.isInteger(minute)) return "a time you choose";
  return new Date(2000, 0, 1, hour, minute).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/** When a schedule runs, to end a sentence: "on Fridays at 4:00 PM". */
function scheduleWords(sc) {
  const at = timeWords(sc?.at);
  const days = HOOK_DAYS.map(([d]) => d).filter((d) => (sc?.days ?? []).includes(d));
  if (!days.length || days.length === 7) return `every day at ${at}`;
  if (days.length === 5 && days.every((d) => HOOK_WEEKDAYS.includes(d))) return `on weekdays at ${at}`;
  return `on ${daysWords(days)} at ${at}`;
}

/** The notes a schedule reads, in words: "the last 7 days". */
function coversWords(sc) {
  return sc?.covers === "week" ? "the last 7 days" : "the last day";
}

/** When a hook before a meeting runs, to end a sentence: "10 minutes before each meeting on your calendar". */
function beforeWords(h) {
  const minutes = h.before?.minutes ?? 10;
  return `${minutes === 1 ? "a minute" : `${minutes} minutes`} before each meeting on your calendar`;
}

/** Where what the AI wrote goes. A dictation or a note from the phone has no note of its own. */
const HOOK_PUTS = [
  ["note", "In the note, as a section of its own"],
  ["other-note", "At the end of another note"],
  ["file", "At the end of a file"],
  ["clipboard", "On the clipboard"],
  ["web", "To a web address"],
];

/** Things to ask the AI that are ready to use, or to start from. */
const HOOK_STARTERS = [
  {
    label: "Follow-up email, after a meeting",
    when: "meeting",
    name: "Follow-up email",
    put: "note",
    instruction:
      "Draft a short follow-up email from me to the other people in this meeting. Start with one line of thanks, then what was decided, then each action item with who will do it and by when (only where the meeting said so). Plain and friendly, ready to send. No subject line.",
  },
  {
    label: "What I said I'd do, after a meeting",
    when: "meeting",
    name: "My commitments",
    put: "other-note",
    to: "Commitments",
    instruction:
      "List everything I (\"You\" in the transcript) said I would do, one per line as a to-do (\"- [ ] \"), with who it's for, and by when if I said so. Only what I took on myself, not what others said they'd do.",
  },
  {
    label: "Decisions, after a meeting",
    when: "meeting",
    name: "Decisions",
    put: "other-note",
    to: "Decisions",
    instruction:
      "List what was decided in this meeting, one line each, with who decided or agreed where the transcript says. Only decisions: no discussion and no action items.",
  },
  {
    label: "Key points, of a transcript",
    when: "transcript",
    name: "Key points",
    put: "note",
    instruction:
      "Write the five to ten most important points of this recording as a list, in the order they come up, each with the time it starts.",
  },
  {
    label: "Study cards, from a transcript",
    when: "transcript",
    name: "Study cards",
    put: "note",
    instruction:
      "Write question-and-answer cards for studying this: one for each fact or idea worth remembering, as a \"Q:\" line and an \"A:\" line, using only what the recording says.",
  },
  {
    label: "To-dos, from a note said on my iPhone",
    when: "phone-note",
    name: "To-dos from my phone",
    put: "other-note",
    to: "To-dos",
    instruction:
      "If this note has things I need to do, list each as a to-do (\"- [ ] \"), keeping names, dates and times as I said them. If it has none, there is nothing to do.",
  },
  {
    label: "Weekly review, on Fridays",
    notify: true,
    when: "schedule",
    schedule: { at: "16:00", days: ["fri"], covers: "week" },
    name: "Weekly review",
    put: "other-note",
    to: "Weekly reviews",
    instruction:
      "Write a review of my week from these notes, under three headings. \"Decided\": what the notes say was decided. \"For me to do\": only what the notes say I (\"You\") will do or said I would do, each as a to-do (\"- [ ] \") with who it's for and by when; things I dictated to myself count. \"Waiting on others\": what someone else said they would do, with their name. Take each note in turn and leave nothing of these out. End each line with the title of the note it comes from, in brackets. Keep days as the notes say them (\"by Wednesday\") and add no dates of your own. Leave out what is already done.",
  },
  {
    label: "Daily wrap-up, on weekdays",
    notify: true,
    when: "schedule",
    schedule: { at: "17:30", days: [...HOOK_WEEKDAYS], covers: "day" },
    name: "Daily wrap-up",
    put: "other-note",
    to: "Daily wrap-ups",
    instruction:
      "From these notes of my day, write two lists, each under its heading. \"For me to do\": only what the notes say I (\"You\") will do or said I would do, each as a to-do (\"- [ ] \") with who it's for and by when; things I dictated to myself count. \"Decided\": what the notes say was decided, one line each. Take each note in turn. End each line with the title of the note it comes from, in brackets. Keep days as the notes say them (\"by Wednesday\") and add no dates of your own. Leave out what is already done.",
  },
  {
    label: "Brief, before a meeting",
    notify: true,
    when: "before-meeting",
    before: { minutes: 10 },
    name: "Brief",
    put: "other-note",
    to: "Briefs",
    instruction:
      "Write a brief for the meeting that is about to start, from the notes of the earlier meetings, under three headings. \"Decided\": what the notes say was decided, one line each. \"For me to do\": only what the notes say I (\"You\") will do or said I would do, each as a to-do (\"- [ ] \") with who it is for and by when. \"Waiting on others\": what someone else said they would do, each line starting with their name. Go through the earlier meetings one at a time, and through each one's decisions and action items one at a time, and put every one under a heading, even one about something else. End each line with the title and day of the meeting it comes from, in brackets, as in \"(Weekly sync, 3 November)\". Keep days as the notes say them (\"by Wednesday\") and add no dates of your own. Leave out what a later meeting says is done or changed.",
  },
];

/** The AI a hook's instruction goes to, as `[name, onThisComputer]`, once known. */
let hookAi = null;

/** Where the text goes when a hook asks the AI, to finish a sentence. */
function hookAiName() {
  if (!hookAi) return "the AI you chose for instructions";
  return hookAi[1] ? "the AI on this computer" : `${hookAi[0]}, in the cloud`;
}

/** A preset's name, from its id. */
function presetName(id) {
  return (state.presets ?? []).find((p) => p.id === id)?.name ?? id;
}

/** What a hook runs, in a few words. */
function hookSummary(h) {
  if (h.kind === "ask") {
    const where = {
      note: "in the note",
      "other-note": `in the note ${h.to}`,
      file: `in ${h.to}`,
      clipboard: "on the clipboard",
      web: `to ${h.to}`,
    }[h.put];
    return `Asks ${hookAi && !hookAi[1] ? hookAi[0] : "the AI"} and puts what it writes ${where}`;
  }
  if (h.kind === "web") return `Sends to ${h.url}`;
  return `Runs ${h.path}${h.args?.length ? ` ${h.args.join(" ")}` : ""}`;
}

/** What happens to a scheduled hook whose time passes unseen. */
const HOOK_LATE = " If the computer is asleep or Splashdown isn't running then, it runs when it next can, with the notes up to that time.";

/** What a hook before a meeting leaves alone, and where the calendar is read. */
const HOOK_CALENDAR = " All-day events and ones you declined are skipped, and a meeting with no earlier notes runs nothing. Your calendar is read on this Mac only; macOS asks first.";

/** What a hook does, in a sentence, for the editor. */
function hookSentence(h) {
  const preset = h.preset ? ` with the ${presetName(h.preset)} preset` : "";
  if (h.kind === "ask") {
    const text = {
      dictation: `each dictation${preset}`,
      meeting: "each meeting's note, with its transcript,",
      transcript: "each finished transcript",
      "phone-note": "each note from your iPhone",
      schedule: `the notes from ${coversWords(h.schedule)} (meetings with their summaries, transcripts and your other notes), ${scheduleWords(h.schedule)},`,
      "before-meeting": `your notes of the last meetings with the same title or the same people, ${beforeWords(h)},`,
    }[h.when] ?? "the text";
    const only = h.title_has ? ` (only when its title has “${h.title_has}”)` : "";
    const put = {
      note: "goes in that note, in a section with this name",
      "other-note": `is added to the end of the note ${h.to || "you name"}, which is made if it isn't there`,
      file: `is added to the end of ${h.to || "the file"}`,
      clipboard: "replaces what's on the clipboard",
      web: `is sent to ${h.to || "the web address"}, signed with your webhook secret`,
    }[h.put];
    const long = hookAi?.[1]
      ? ` ${h.when === "schedule" || h.when === "before-meeting" ? "Many notes are" : "A long meeting is"} read a part at a time there, which can take a few minutes.`
      : "";
    const late = h.when === "schedule" ? HOOK_LATE : h.when === "before-meeting" ? HOOK_CALENDAR : "";
    const told = h.notify ? " A small panel tells you when it's done." : "";
    return `Splashdown gives ${text}${only} to ${hookAiName()} with what you wrote above. What it writes ${put}.${told} It can only write: it can't send, open or change anything else.${long}${late}`;
  }
  const what = h.kind === "web"
    ? `Splashdown sends the text and its details to ${h.url || "the web address"}, signed with your webhook secret.`
    : `Splashdown runs ${h.path || "the program"} with the text as its input, never through a shell.`;
  const when = {
    dictation: ` After every dictation${preset} that goes into a text field.`,
    "before-insert": ` Before each dictation${preset} goes in: what it writes back is inserted instead. If it takes over 2 seconds or fails, the dictation goes in as said.`,
    meeting: " When a meeting's note is finished, with its summary.",
    transcript: " When a transcript of a file, a link, an app's audio or the microphone is finished.",
    "phone-note": " For each note from your iPhone.",
    schedule: ` ${scheduleWords(h.schedule).replace(/^./, (c) => c.toUpperCase())}, with the notes from ${coversWords(h.schedule)} as the text.${HOOK_LATE}`,
    "before-meeting": ` ${beforeWords(h).replace(/^./, (c) => c.toUpperCase())}, with the meeting and your notes of the last meetings with the same title or the same people as the text.${HOOK_CALENDAR}`,
  }[h.when];
  const only = h.title_has ? ` Only when the title has “${h.title_has}”.` : "";
  return what + when + only;
}

/** The hooks, each with Turn off or on, Edit and Delete. */
function renderHooks(hooks) {
  $("hk-secret-row").hidden = !hooks.some((h) => h.kind === "web" || (h.kind === "ask" && h.put === "web"));
  $("hk-list").replaceChildren(
    ...hooks.map((h, i) => {
      const on = h.on !== false;
      const row = document.createElement("div");
      row.className = "row";
      row.classList.toggle("dim", !on);
      const lbl = document.createElement("span");
      lbl.className = "lbl";
      const timed = h.when === "schedule" ? scheduleWords(h.schedule) : h.when === "before-meeting" ? beforeWords(h) : null;
      const when = timed ?? HOOK_EVENTS.find(([v]) => v === h.when)?.[1] ?? h.when;
      const only = h.preset ? ` · ${presetName(h.preset)}` : h.title_has ? ` · “${h.title_has}”` : "";
      const shown = timed ? when : when.toLowerCase();
      lbl.textContent = (h.kind === "ask" ? `${h.name} · ${shown}` : when.replace(/^./, (c) => c.toUpperCase())) + only;
      const small = document.createElement("small");
      small.textContent = hookSummary(h) + (on ? "" : " · off");
      lbl.append(small);
      const buttons = document.createElement("span");
      buttons.className = "ma-buttons";
      // One with notes of its own to run on needn't wait for its time.
      if (on && (h.when === "schedule" || h.when === "before-meeting")) {
        const now = button("Run now", async () => {
          now.disabled = true;
          try {
            toast(await invoke("hooks_run_now", { hook: h }));
          } catch (e) {
            toast(String(e), true);
          } finally {
            now.disabled = false;
          }
        }, "link");
        buttons.append(now);
      }
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

/** What the hooks did lately, newest first; the last few until Show all. */
const HOOK_RUNS_SHOWN = 5;
let hookRunsAll = false;

async function loadHookRuns() {
  const runs = (await invoke("hooks_runs").catch(() => [])) ?? [];
  $("hk-runs-row").hidden = !runs.length;
  const shown = hookRunsAll ? runs : runs.slice(0, HOOK_RUNS_SHOWN);
  $("hk-runs").replaceChildren(
    ...shown.map((r) => {
      const line = document.createElement("div");
      line.className = "hk-run";
      line.classList.toggle("warn-text", !r.ok);
      const at = new Date(r.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
      const took = r.took >= 10 ? ` (${Math.round(r.took)} s)` : "";
      line.textContent = `${at} · ${r.hook}${r.about ? ` · ${r.about}` : ""}: ${r.said}${took}`;
      return line;
    }),
  );
  $("hk-runs-all").hidden = hookRunsAll || runs.length <= HOOK_RUNS_SHOWN;
}

listen("hooks-ran", () => loadHookRuns());
$("hk-runs-all").addEventListener("click", () => {
  hookRunsAll = true;
  loadHookRuns();
});
$("hk-runs-clear").addEventListener("click", async () => {
  await invoke("hooks_runs_clear").catch((e) => toast(String(e), true));
  hookRunsAll = false;
  loadHookRuns();
});

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
  const h = index === null ? { when: "meeting", kind: "ask", name: "", instruction: "", put: "note" } : hooks[index];
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
  // The calendar is only read on a Mac.
  const beforeMeeting = when.querySelector('option[value="before-meeting"]');
  if (!mac) {
    beforeMeeting.disabled = true;
    beforeMeeting.textContent += " (on a Mac)";
  }
  when.value = h.when;
  const preset = document.createElement("select");
  const presetPart = field("Preset", preset);
  const kind = document.createElement("select");
  kind.append(
    new Option("Run a script or program", "program"),
    new Option("Send to a web address", "web"),
    new Option("Ask the AI to do something with it", "ask"),
  );
  kind.value = h.kind;
  const titleHas = text(h.title_has, "1:1");
  const titlePart = field("Only when the title has", titleHas, "Leave empty for all of them.");
  // On a schedule: the time, the days and how far back its notes go.
  const at = document.createElement("input");
  at.type = "time";
  at.value = h.schedule?.at?.padStart(5, "0") ?? "16:00";
  const days = document.createElement("select");
  days.append(
    new Option("Every day", "every"),
    new Option("Weekdays", "weekdays"),
    ...HOOK_DAYS.map(([d, name]) => new Option(`${name}s`, d)),
  );
  // Days set by hand in the settings file that the list can't say.
  let otherDays = null;
  const showDays = (list) => {
    const set = HOOK_DAYS.map(([d]) => d).filter((d) => (list ?? []).includes(d));
    days.querySelector('option[value="other"]')?.remove();
    otherDays = null;
    if (!set.length || set.length === 7) days.value = "every";
    else if (set.length === 5 && set.every((d) => HOOK_WEEKDAYS.includes(d))) days.value = "weekdays";
    else if (set.length === 1) days.value = set[0];
    else {
      otherDays = set;
      days.append(new Option(daysWords(set), "other"));
      days.value = "other";
    }
  };
  const chosenDays = () =>
    days.value === "every" ? [] : days.value === "weekdays" ? [...HOOK_WEEKDAYS] : days.value === "other" ? otherDays ?? [] : [days.value];
  const covers = document.createElement("select");
  for (const [value, label] of HOOK_COVERS) covers.append(new Option(label, value));
  covers.value = h.schedule?.covers ?? "day";
  showDays(h.schedule?.days);
  const schedulePart = document.createElement("div");
  schedulePart.append(
    field("At", at, "This computer's time."),
    field("On", days),
    field("With the notes from", covers, "Meetings come with their summaries. Notes hidden from coding agents are left out."),
  );
  // Before a meeting: how long before.
  const minutes = document.createElement("input");
  minutes.type = "number";
  minutes.min = "1";
  minutes.max = "120";
  minutes.value = String(h.before?.minutes ?? 10);
  const beforePart = field("Minutes before it starts", minutes, "Long enough for the AI to write: a minute or two on this computer.");
  // Asking the AI.
  const starter = document.createElement("select");
  starter.append(
    new Option("Your own", ""),
    ...HOOK_STARTERS.map((st, i) => Object.assign(new Option(st.label, String(i)), { disabled: st.when === "before-meeting" && !mac })),
  );
  const askName = text(h.name, "Follow-up email");
  const instruction = document.createElement("textarea");
  instruction.className = "ma-instruction";
  instruction.value = h.instruction ?? "";
  instruction.placeholder = "Draft a short follow-up email to the people in this meeting…";
  const put = document.createElement("select");
  const to = text(h.kind === "ask" ? h.to : "", "");
  const toPart = field("", to);
  // Tell me when it's done: a panel with how it starts and a way to open it.
  const notify = document.createElement("input");
  notify.type = "checkbox";
  notify.checked = !!h.notify;
  const notifyPart = document.createElement("label");
  notifyPart.className = "ma-check";
  notifyPart.append(notify, "Tell me when it's done", Object.assign(document.createElement("small"), { textContent: " A small panel shows how it starts, with Open." }));
  const askPart = document.createElement("div");
  askPart.append(
    field("Start from", starter),
    field("Name", askName, "The heading what it writes goes under."),
    field("What the AI should do", instruction, "In your own words."),
    field("Put what it writes", put),
    toPart,
    notifyPart,
  );
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
  result.className = "ma-note ma-result";
  result.hidden = true;
  const error = document.createElement("p");
  error.className = "ma-error";
  error.hidden = true;
  // The hook as the form has it now.
  const current = () => {
    const hook = { when: when.value, kind: kind.value, on: h.on !== false };
    if (when.value === "schedule") {
      hook.schedule = { at: at.value, covers: covers.value };
      if (chosenDays().length) hook.schedule.days = chosenDays();
    }
    if (when.value === "before-meeting") hook.before = { minutes: Math.round(Number(minutes.value)) || 0 };
    if (preset.value) hook.preset = preset.value;
    if (!titlePart.hidden && titleHas.value.trim()) hook.title_has = titleHas.value.trim();
    if (kind.value === "web") hook.url = url.value.trim();
    else if (kind.value === "ask") {
      hook.name = askName.value.trim();
      hook.instruction = instruction.value.trim();
      hook.put = put.value;
      if (!toPart.hidden) hook.to = to.value.trim();
      if (notify.checked) hook.notify = true;
    } else {
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
    kind.options[2].disabled = changes;
    if (changes) kind.value = "program";
    webPart.hidden = kind.value !== "web";
    programPart.hidden = kind.value !== "program";
    askPart.hidden = kind.value !== "ask";
    titlePart.hidden = !["meeting", "transcript", "before-meeting"].includes(when.value);
    schedulePart.hidden = when.value !== "schedule";
    beforePart.hidden = when.value !== "before-meeting";
  };
  // Where the result can go for this event, and what that place needs.
  const showPut = () => {
    const hasNote = when.value === "meeting" || when.value === "transcript";
    const kept = put.value || (h.kind === "ask" ? h.put : "") || "";
    put.replaceChildren(...HOOK_PUTS.filter(([v]) => hasNote || v !== "note").map(([v, label]) => new Option(label, v)));
    put.value = [...put.options].some((o) => o.value === kept) ? kept : put.options[0].value;
    showTo();
  };
  const showTo = () => {
    const [label, placeholder, note] = {
      "other-note": ["Note", "Commitments", "A note in your notes folder, made if it isn't there."],
      file: ["File", mac ? "/Users/you/Documents/meeting-log.md" : "C:\\Users\\you\\Documents\\meeting-log.md", "Its full path."],
      web: ["Web address", "https://hooks.zapier.com/hooks/catch/…", "What it writes is sent as JSON."],
    }[put.value] ?? [];
    toPart.hidden = !label;
    if (!label) return;
    toPart.querySelector("label").replaceChildren(label, Object.assign(document.createElement("small"), { textContent: ` ${note}` }));
    to.placeholder = placeholder;
  };
  const refresh = () => {
    showPresets();
    showKind();
    showPut();
    describe();
    result.hidden = true;
  };
  when.addEventListener("change", refresh);
  kind.addEventListener("change", refresh);
  preset.addEventListener("change", describe);
  put.addEventListener("change", () => {
    to.value = "";
    showTo();
    describe();
  });
  starter.addEventListener("change", () => {
    const st = HOOK_STARTERS[Number(starter.value)];
    if (!starter.value || !st) return;
    when.value = st.when;
    if (st.schedule) {
      at.value = st.schedule.at;
      covers.value = st.schedule.covers;
      showDays(st.schedule.days);
    }
    if (st.before) minutes.value = String(st.before.minutes);
    askName.value = st.name;
    instruction.value = st.instruction;
    notify.checked = !!st.notify;
    refresh();
    put.value = st.put;
    to.value = st.to ?? "";
    showTo();
    describe();
  });
  for (const input of [url, path, args, titleHas, askName, instruction, to, at, minutes]) input.addEventListener("input", describe);
  for (const input of [days, covers, notify]) input.addEventListener("change", describe);
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
    const about = {
      schedule: `your notes from ${coversWords(hook.schedule)}`,
      "before-meeting": "your next meeting",
    }[hook.when] ?? "a sample";
    result.textContent = hook.kind === "ask" ? `Asking ${hookAiName()} about ${about}…` : "Testing…";
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
    titlePart,
    schedulePart,
    beforePart,
    field("What it does", kind),
    webPart,
    programPart,
    askPart,
    sentence,
    result,
    error,
    acts,
  );
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const hook = current();
    if (!(await check(hook))) return;
    // A hook before a meeting needs the calendar: macOS asks now, while
    // the user is here to answer.
    if (hook.when === "before-meeting" && !(await invoke("calendar_access").catch(() => false))) {
      error.textContent = "macOS didn't allow Splashdown to read your calendar. Allow it in System Settings → Privacy & Security → Calendars, then save again.";
      error.hidden = false;
      return;
    }
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
