"use strict";

// Transcribing an app's audio or the microphone into a note (picking the
// app); also opening transforms to edit, and a note's earlier versions.

/** Icons for the apps in the picker, by id, once looked up. */
const appIcons = new Map();

async function pickApp() {
  if (meeting) return toast(meeting.app ? "Already recording." : "A meeting is recording.");
  const dialog = $("nt-apps");
  if (!dialog.open) dialog.showModal();
  await listApps();
}

async function listApps() {
  const list = $("nt-apps-list");
  list.replaceChildren(Object.assign(document.createElement("p"), { className: "nt-hint", textContent: "Looking for apps that play sound…" }));
  let apps = [];
  try {
    apps = (await invoke("audio_apps")) ?? [];
  } catch (e) {
    toast(String(e), true);
  }
  const everything = { id: "", name: "All of this computer's sound", playing: false, sources: [], all: true };
  const rows = [...apps, everything].map((a) => {
    const b = document.createElement("button");
    b.className = "nt-app";
    b.setAttribute("role", "option");
    const iconUrl = appIcons.get(a.id);
    if (iconUrl) {
      b.append(Object.assign(document.createElement("img"), { src: iconUrl, alt: "" }));
    } else {
      const ic = document.createElement("span");
      ic.className = "nt-app-ic";
      ic.dataset.app = a.id;
      ic.append(icon("sound"));
      b.append(ic);
    }
    const name = document.createElement("span");
    name.className = "nt-app-name";
    name.textContent = a.name;
    b.append(name);
    if (!a.all) {
      const state = document.createElement("span");
      state.className = `nt-app-state${a.playing ? " on" : ""}`;
      state.textContent = a.playing ? "Playing" : "Not playing now";
      b.append(state);
    }
    b.addEventListener("click", () => startApp(a));
    return b;
  });
  if (!apps.length) {
    rows.unshift(Object.assign(document.createElement("p"), {
      className: "nt-hint",
      textContent: "No app has played sound yet. Start playing something, then look again.",
    }));
  }
  list.replaceChildren(...rows);
  list.querySelector("button")?.focus();
  // Icons, once each.
  const missing = apps.map((a) => a.id).filter((id) => id && !appIcons.has(id));
  if (missing.length) {
    invoke("app_info", { ids: missing })
      .then((infos) => {
        for (const info of infos ?? []) {
          if (!info.icon) continue;
          appIcons.set(info.id, info.icon);
          for (const slot of list.querySelectorAll(`.nt-app-ic[data-app="${CSS.escape(info.id)}"]`)) {
            slot.replaceWith(Object.assign(document.createElement("img"), { src: info.icon, alt: "" }));
          }
        }
      })
      .catch(() => {});
  }
}

/** Transcribes the microphone alone into a note, hands-free. */
async function recordMicrophone() {
  if (meeting) return toast(meeting.app ? "Already recording." : "A meeting is recording.");
  try {
    await invoke("meeting_start_microphone");
  } catch (e) {
    toast(String(e), true);
  }
}

async function startApp(a) {
  $("nt-apps").close();
  try {
    await invoke("meeting_start_app", { name: a.all ? "This computer's sound" : a.name, sources: a.sources });
  } catch (e) {
    toast(String(e), true);
  }
}

$("nt-apps-refresh").addEventListener("click", listApps);
$("nt-apps-cancel").addEventListener("click", () => $("nt-apps").close());

function editRewrites() {
  call("notes_edit_transforms");
}

async function showHistory(show) {
  if (show && (!current.path || locked())) return;
  if (show) closeReview();
  $("nt-editor").hidden = show;
  $("nt-history").hidden = !show;
  if (!show) return focusNote();
  await saveNow();
  let versions = [];
  try {
    versions = await invoke("notes_versions", { path: current.path });
  } catch (e) {
    toast(String(e), true);
  }
  $("nt-versions-list").replaceChildren(...versions.map(versionRow));
  $("nt-versions-empty").hidden = versions.length > 0;
  $("nt-history-back").focus();
}

function versionRow(v) {
  const li = document.createElement("li");
  const preview = document.createElement("div");
  preview.className = "nt-preview";
  preview.textContent = v.preview;
  const meta = document.createElement("div");
  meta.className = "nt-vmeta";
  const time = document.createElement("time");
  time.dateTime = new Date(v.at).toISOString();
  const d = new Date(v.at);
  time.textContent = `${when(v.at)}${d.toDateString() === new Date().toDateString() ? "" : `, ${d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`}`;
  const restore = button("Restore", async () => {
    try {
      showNote(await invoke("notes_restore", { path: current.path, at: v.at }));
      toast("Restored. The version it replaced is in Versions.");
      await refresh();
    } catch (e) {
      toast(String(e), true);
    }
  }, "link");
  restore.setAttribute("aria-label", `Restore the version from ${time.textContent}`);
  meta.append(time, restore);
  li.append(preview, meta);
  return li;
}
