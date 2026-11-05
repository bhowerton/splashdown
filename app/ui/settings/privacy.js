"use strict";

// Privacy tab: where speech and text are sent, local-only mode, what the AI
// is told, and what's kept on this computer.

function renderPrivacy() {
  const s = state.settings;
  const dest = (kind, el, badgeEl, resolvedText) => {
    const label = resolvedText.split(" · ")[0];
    $(el).textContent = resolvedText;
    const b = document.createElement("span");
    if (kind === "local") {
      b.className = "badge ok";
      b.textContent = "On this computer";
    } else if (kind === "cloud") {
      b.className = "badge warn";
      b.textContent = `Sent to ${label}`;
    } else {
      b.className = "badge muted";
      b.textContent = "Off";
    }
    $(badgeEl).replaceChildren(b);
  };
  dest(state.speech_destination, "pv-speech", "pv-speech-badge", resolved(state.resolved_speech));
  dest(state.ai_destination, "pv-ai", "pv-ai-badge", resolved(state.resolved_ai));
  if (state.resolved_instructions) {
    $("pv-ai").textContent += ` · instructions: ${state.resolved_instructions}`;
    $("pv-ai-badge").append(badge(`Instructions sent to ${state.resolved_instructions.split(" · ")[0]}`, "warn"));
  }
  const withContext = (state.presets ?? []).filter((p) => p.field_context).map((p) => p.name);
  $("pv-context").textContent = withContext.length
    ? `Sent for reference with the ${withContext.join(", ")} preset${withContext.length === 1 ? "" : "s"} (up to 1,000 characters).`
    : "Never sent. Presets can turn it on, so names are spelled like the conversation.";
  const withClipboard = (state.presets ?? []).filter((p) => p.clipboard_context).map((p) => p.name);
  $("pv-clipboard").textContent = withClipboard.length
    ? `What you last copied is sent for reference with the ${withClipboard.join(", ")} preset${withClipboard.length === 1 ? "" : "s"} (up to 2,000 characters).`
    : "Never sent. Presets can turn it on, for names and code you've copied.";
  renderNotesFolder();
  $("pv-local").checked = !!s.general.local_only;
  $("pv-appname").checked = s.general.share_app_name !== false;
  $("pv-recent").checked = s.general.share_recent !== false;
  $("pv-window").checked = s.general.compose_reads_window !== false;
  $("pv-screen-words").checked = s.general.screen_words !== false;
  $("pv-ask-notes").checked = s.notes?.ask_uses_notes !== false;
  $("pv-agents-read").checked = !!s.notes?.agents_read;
  $("pv-agents-write").checked = !!s.notes?.agents_write;
  $("pv-agents-write").disabled = !s.notes?.agents_read;
  // Unset (from before this switch) means on for whoever had allowed
  // notes; someone turning notes on for the first time starts with it off.
  $("pv-agents-meetings").checked = s.notes?.agents_meetings ?? !!s.notes?.agents_read;
  $("pv-agents-meetings").disabled = !s.notes?.agents_read;
  const days = s.general.history_days;
  const kept = days ? `, deleted after ${days === 1 ? "1 day" : `${days} days`}` : "";
  $("pv-history").textContent = s.general.keep_history === true
    ? `On: the last 300 dictations${s.general.keep_audio ? " and their recordings" : ""}${kept}, in this app's settings folder.`
    : `Off (the default). Only the last result is kept in memory, for “Paste last raw transcript”.${days ? ` Anything saved earlier is deleted after ${days === 1 ? "1 day" : `${days} days`}.` : ""}`;
  $("pv-keys").textContent = state.platform === "macos"
    ? "Stored in your macOS Keychain. Keys in a .env.local file (for development) override it."
    : "Stored in Windows Credential Manager.";
}

// "Manage" / "Change" links that jump to another tab.
document.querySelectorAll("[data-goto]").forEach((b) => b.addEventListener("click", () => showTab(b.dataset.goto)));
$("nt-open").addEventListener("click", () => invoke("open_notes").catch((e) => toast(String(e), true)));
$("pv-notes-open").addEventListener("click", () => invoke("open_notes").catch((e) => toast(String(e), true)));

/** Where notes are kept, and whether a sync service has a copy. Asking
 *  doesn't create the folder. */
async function renderNotesFolder() {
  const s = state.settings;
  let folder;
  try {
    folder = await invoke("notes_folder");
  } catch {
    return;
  }
  if (!folder) return;
  const el = $("pv-notes");
  el.textContent = `What you write in Notes (${comboLabel("N", state.platform)}), one Markdown file per note, in ${folder.label}. Never sent anywhere unless you send or copy it${s.notes?.agents_read ? ", or a coding agent you've allowed reads it" : ""}.`;
  el.title = folder.path;
  if (folder.synced) {
    el.textContent += ` That folder is in ${folder.synced}, so ${folder.synced} keeps a copy.`;
  }
  // Synced notes stand out while local-only mode is on.
  el.classList.toggle("warn-text", !!folder.synced && !!state.settings.general.local_only);
}

/** Settings → Privacy → Coding agents → Set up an agent: each agent this
 *  computer has, whether Splashdown is set up in it, and a button. */
async function renderCodingAgents() {
  const list = $("pv-agents-list");
  let found;
  try {
    found = await invoke("coding_agents");
  } catch (e) {
    list.replaceChildren(agentRow("Couldn't check for agents", String(e)));
    return;
  }
  $("pv-agents-server").textContent = ` For any other agent, its command is ${found.server}.`;
  const rows = found.agents.map((a) => {
    const only = a.only_in ?? [];
    const state = a.set_up
      ? a.points_here === false
        ? "Set up, with another copy of Splashdown. Set it up again to use this one."
        : "Set up."
      : only.length
        ? `Set up for ${only.join(", ")} only.`
        : a.found
          ? "Not set up."
          : "Not found on this computer.";
    const row = agentRow(a.name, state);
    const acts = document.createElement("span");
    acts.className = "row-acts";
    if (a.how === "config") {
      // Claude Desktop: lines to add to its config by hand.
      acts.append(agentButton("Copy setup", () => copyForAgent(a.snippet, `Copied. In Claude Desktop: Settings → Developer → Edit Config, add it, then restart Claude Desktop.`)));
    } else {
      const again = a.set_up && a.points_here !== false;
      acts.append(agentButton(again ? "Set up again" : "Set up", () => setUpAgent(a), !a.found && a.how === "command" && a.id !== "vscode"));
      if (a.command) acts.append(agentButton("Copy command", () => copyForAgent(a.command, "Copied. Run it in a terminal."), false, "link"));
    }
    row.append(acts);
    return row;
  });
  list.replaceChildren(...rows);
}

function agentRow(name, detail) {
  const row = document.createElement("div");
  row.className = "row";
  const lbl = document.createElement("span");
  lbl.className = "lbl";
  lbl.textContent = name;
  const small = document.createElement("small");
  small.textContent = detail;
  lbl.append(small);
  row.append(lbl);
  return row;
}

function agentButton(text, onClick, disabled = false, cls = "btn") {
  const b = document.createElement("button");
  b.type = "button";
  b.className = cls;
  b.textContent = text;
  b.disabled = disabled;
  b.addEventListener("click", onClick);
  return b;
}

async function copyForAgent(text, message) {
  try {
    await invoke("copy_text", { text });
    toast(message);
  } catch (e) {
    toast(String(e), true);
  }
}

async function setUpAgent(agent) {
  try {
    toast(await invoke("coding_agent_install", { id: agent.id }));
  } catch (e) {
    toast(String(e), true);
  }
  renderCodingAgents();
}
