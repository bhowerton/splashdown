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
  $("pv-agents-read").checked = !!s.notes?.agents_read;
  $("pv-agents-write").checked = !!s.notes?.agents_write;
  $("pv-agents-write").disabled = !s.notes?.agents_read;
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
  el.textContent = `What you write in Notes (⌃⌥N), one Markdown file per note, in ${folder.label}. Never sent anywhere unless you send or copy it${s.notes?.agents_read ? ", or a coding agent you've allowed reads it" : ""}.`;
  el.title = folder.path;
  if (folder.synced) {
    el.textContent += ` That folder is in ${folder.synced}, so ${folder.synced} keeps a copy.`;
  }
  // Synced notes stand out while local-only mode is on.
  el.classList.toggle("warn-text", !!folder.synced && !!state.settings.general.local_only);
}
