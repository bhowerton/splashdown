"use strict";

// Snippets and find-and-replace rules (on the Words & language tab).
// Snippets: spoken trigger phrases that insert saved text, saved straight
// into settings.snippets on every change.

function renderSnippets() {
  const host = $("snippets");
  // Don't rebuild while the user is typing in it.
  if (typingIn(host)) return;
  host.replaceChildren(
    ...state.settings.snippets.map((sn, i) => {
      const row = document.createElement("div");
      row.className = "snippet";
      const trigger = document.createElement("input");
      trigger.type = "text";
      trigger.setAttribute("aria-label", "Snippet trigger phrase");
      trigger.placeholder = "Trigger, e.g. my address";
      trigger.value = sn.trigger;
      const text = document.createElement("textarea");
      text.rows = 1;
      text.setAttribute("aria-label", "Snippet text to insert");
      text.placeholder = "Text to insert";
      text.value = sn.text;
      const commit = () => saveSnippets((list) => (list[i] = { trigger: trigger.value.trim(), text: text.value }));
      trigger.onchange = commit;
      text.onchange = commit;
      const x = document.createElement("button");
      x.className = "x";
      x.textContent = "×";
      x.title = "Remove";
      x.setAttribute("aria-label", `Remove snippet ${sn.trigger || ""}`.trim());
      x.onclick = () => saveSnippets((list) => list.splice(i, 1));
      const wrap = document.createElement("div");
      wrap.className = "rule";
      wrap.append(x);
      row.append(trigger, text, wrap);
      return row;
    })
  );
  // Two snippets with the same trigger: only the first is used.
  const seen = new Set();
  const dupes = new Set();
  state.settings.snippets.forEach((sn) => {
    const t = sn.trigger.trim().toLowerCase();
    if (!t) return;
    if (seen.has(t)) dupes.add(sn.trigger.trim());
    seen.add(t);
  });
  const warn = $("snippet-warn");
  warn.hidden = dupes.size === 0;
  warn.textContent = dupes.size ? `More than one snippet uses “${[...dupes].join("”, “")}”; only the first is used.` : "";
}

function saveSnippets(change) {
  const s = collect();
  s.snippets = structuredClone(state.settings.snippets);
  change(s.snippets);
  // Half-filled rows are kept while editing; empty ones are dropped on save.
  s.snippets = s.snippets.filter((sn) => sn.trigger || sn.text);
  state.settings.snippets = s.snippets;
  save(s);
}

// ---------------------------------------------------------------- replacements

function renderReplacements() {
  const host = $("replacements");
  if (typingIn(host)) return;
  host.replaceChildren(
    ...(state.settings.replacements ?? []).map((r, i) => {
      const row = document.createElement("div");
      row.className = "replacement";
      const find = document.createElement("input");
      find.type = "text";
      find.spellcheck = false;
      find.placeholder = "Find";
      find.value = r.find;
      find.setAttribute("aria-label", "Find");
      const arrow = document.createElement("span");
      arrow.className = "arrow";
      arrow.textContent = "→";
      arrow.setAttribute("aria-hidden", "true");
      const replace = document.createElement("input");
      replace.type = "text";
      replace.spellcheck = false;
      replace.placeholder = "Replace with";
      replace.value = r.replace;
      replace.setAttribute("aria-label", "Replace with");
      const regexLabel = document.createElement("label");
      regexLabel.className = "regex-toggle";
      const regex = document.createElement("input");
      regex.type = "checkbox";
      regex.checked = !!r.regex;
      regexLabel.append(regex, document.createTextNode("Regex"));
      const commit = () => saveReplacements((list) => (list[i] = { find: find.value, replace: replace.value, regex: regex.checked }));
      find.onchange = commit;
      replace.onchange = commit;
      regex.onchange = commit;
      const x = document.createElement("button");
      x.className = "x";
      x.textContent = "×";
      x.setAttribute("aria-label", `Remove replacement ${r.find}`.trim());
      x.onclick = () => saveReplacements((list) => list.splice(i, 1));
      row.append(find, arrow, replace, regexLabel, x);
      return row;
    })
  );
}

async function saveReplacements(change) {
  const s = collect();
  s.replacements = structuredClone(state.settings.replacements ?? []);
  change(s.replacements);
  s.replacements = s.replacements.filter((r) => r.find || r.replace);
  state.settings.replacements = s.replacements;
  // A bad pattern is refused by the backend; say so next to the list.
  try {
    const r = await invoke("save_settings", { settings: s });
    state = r.state;
    $("replacement-warn").hidden = true;
    render();
    toast("Saved");
  } catch (e) {
    $("replacement-warn").textContent = String(e);
    $("replacement-warn").hidden = false;
  }
}

$("replacement-add").addEventListener("click", () => {
  state.settings.replacements = [...(state.settings.replacements ?? []), { find: "", replace: "", regex: false }];
  renderReplacements();
  $("replacements").querySelector(".replacement:last-child input")?.focus();
});

$("snippet-add").addEventListener("click", () => {
  state.settings.snippets.push({ trigger: "", text: "" });
  renderSnippets();
  $("snippets").querySelector(".snippet:last-child input")?.focus();
});
