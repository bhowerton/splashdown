"use strict";

// History tab: search, list, redo with another preset, transcribe again,
// copy, save to notes, pin and delete.
// (The "Keep a history" switch is a normal setting, drawn in general.js.)

/** The only place mode ids get display labels. The ids are written by the
 *  backend (session.rs, history::Entry.mode); unknown ones are shown as-is. */
const MODE_LABEL = { dictate: "Dictate", dictate_instruct: "Instruct", transform: "Transform", raw: "Raw" };

async function loadHistory() {
  let entries = [];
  try {
    entries = await invoke("get_history", { query: $("hist-search").value });
  } catch (e) {
    toast(String(e), true);
  }
  const off = state?.settings.general.keep_history !== true;
  $("hist-empty").hidden = entries.length > 0 || off;
  $("hist-off").hidden = !off;
  $("hist-list").replaceChildren(...entries.map(historyRow));
}

function historyRow(e) {
  const row = document.createElement("div");
  row.className = "he";
  const meta = document.createElement("div");
  meta.className = "meta";
  const when = new Date(e.at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
  const app = document.createElement("b");
  app.textContent = e.app ?? "Unknown app";
  const mode = document.createElement("span");
  mode.className = "mode-tag";
  mode.textContent = MODE_LABEL[e.mode] ?? e.mode;
  const cost = e.cost_usd ? ` · ${money(e.cost_usd)}` : "";
  meta.append(app, mode, document.createTextNode(`${when} · ${(e.millis / 1000).toFixed(1)} s${cost}`));
  if (e.pinned) {
    const pin = document.createElement("span");
    pin.className = "badge ok";
    pin.textContent = "pinned";
    pin.title = "Kept until you delete it";
    meta.append(pin);
  }
  if (e.note) {
    const note = document.createElement("span");
    note.className = "badge warn";
    note.textContent = "raw text used";
    note.title = e.note;
    meta.append(note);
  }

  const text = document.createElement("div");
  text.className = "text";
  text.textContent = e.text;
  row.append(meta, text);
  if (e.raw && e.raw !== e.text) {
    const raw = document.createElement("div");
    raw.className = "raw";
    raw.textContent = e.instruction ? `${e.raw}  →  ${e.instruction}` : e.raw;
    row.append(raw);
  }

  const acts = document.createElement("div");
  acts.className = "acts";
  if (e.mode !== "transform" && e.raw) {
    const redo = document.createElement("select");
    redo.title = "Run the AI again on what you said, with another preset";
    redo.setAttribute("aria-label", "Redo this dictation with another preset");
    redo.append(new Option("Redo as…", ""), ...state.presets.map((p) => new Option(p.name, p.id)));
    redo.onchange = async () => {
      if (!redo.value) return;
      const out = row.querySelector(".redo-out") ?? row.appendChild(Object.assign(document.createElement("div"), { className: "redo-out" }));
      out.textContent = "Working…";
      try {
        const text = await invoke("redo_history", { at: e.at, presetId: redo.value });
        out.textContent = text;
        const copyBtn = button("Copy", () => copy(text));
        copyBtn.style.marginTop = "6px";
        out.append(document.createElement("br"), copyBtn);
      } catch (err) {
        out.textContent = String(err);
      }
      redo.value = "";
    };
    acts.append(redo);
  }
  if (e.audio && e.mode !== "transform") {
    // Runs the kept recording through the current speech settings again.
    const again = button("Transcribe again", async () => {
      const out = row.querySelector(".redo-out") ?? row.appendChild(Object.assign(document.createElement("div"), { className: "redo-out" }));
      out.textContent = "Transcribing…";
      again.disabled = true;
      try {
        const r = await invoke("retranscribe_history", { at: e.at });
        out.textContent = `${r.text}  (${r.stt})`;
        const copyBtn = button("Copy", () => copy(r.text));
        copyBtn.style.marginTop = "6px";
        out.append(document.createElement("br"), copyBtn);
      } catch (err) {
        out.textContent = String(err);
      }
      again.disabled = false;
    });
    again.title = "Transcribe the kept recording again with your current speech-to-text settings";
    acts.append(again);
  }
  acts.append(
    button("Copy", () => copy(e.text)),
    ...(e.raw && e.raw !== e.text ? [button("Copy what I said", () => copy(e.raw))] : []),
    button("Save to notes", async () => {
      try {
        toast(`Added to ${await invoke("notes_add", { text: e.text })}`);
      } catch (err) {
        toast(String(err), true);
      }
    }),
    button(e.pinned ? "Unpin" : "Pin", async () => {
      await call("pin_history", { at: e.at, pinned: !e.pinned });
      loadHistory();
    }),
    button("Delete", async () => {
      await call("delete_history", { at: e.at });
      loadHistory();
    }, "btn danger")
  );
  row.append(acts);
  return row;
}

async function copy(text) {
  try {
    await invoke("copy_text", { text });
    toast("Copied");
  } catch (e) {
    toast(String(e), true);
  }
}

/** Asks JSON or Markdown, then saves every entry to Downloads. */
$("hist-export").addEventListener("click", () => {
  const dialog = document.createElement("dialog");
  dialog.className = "confirm";
  const title = document.createElement("p");
  title.className = "confirm-title";
  title.textContent = "Export your history";
  const detail = document.createElement("p");
  detail.className = "confirm-detail";
  detail.textContent = "Every saved dictation, with its time, app, mode, what you said and the final text, as one file in Downloads. Markdown is easy to read; JSON is for other apps.";
  const acts = document.createElement("div");
  acts.className = "confirm-acts";
  const exportAs = (format) => async () => {
    dialog.close();
    try {
      const path = await invoke("export_history", { format });
      toast(`Saved to ${path.split(/[\\/]/).pop()} in Downloads`);
    } catch (e) {
      toast(String(e), true);
    }
  };
  const cancel = button("Cancel", () => dialog.close());
  acts.append(cancel, button("JSON", exportAs("json")), button("Markdown", exportAs("markdown"), "btn primary"));
  dialog.append(title, detail, acts);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  cancel.focus();
});

let searchTimer;
$("hist-search").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(loadHistory, 200);
});
$("hist-clear").addEventListener("click", async () => {
  const sure = await askConfirm("Delete all dictation history?", {
    action: "Delete all",
    detail: "Every saved dictation and recording on this computer is removed, pinned ones too. This can't be undone.",
  });
  if (!sure) return;
  await call("clear_history");
  loadHistory();
});
