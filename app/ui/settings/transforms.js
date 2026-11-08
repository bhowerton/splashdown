"use strict";

// Transforms (on the Words & language tab): saved instructions run over a note or
// selected text without speaking. The built-ins are used
// until the list is edited; then the whole list is saved.

function renderTransforms() {
  const host = $("transforms");
  // Don't rebuild while the user is typing in it, or recording a shortcut.
  if (typingIn(host) || shortcutCapture) return;
  host.replaceChildren(...(state.transforms ?? []).map(transformCard));
  $("transform-reset").hidden = !state.settings.transforms;
}

function transformCard(t, i) {
  const card = document.createElement("div");
  card.className = "transform";

  const top = document.createElement("div");
  top.className = "transform-top";
  const name = document.createElement("input");
  name.type = "text";
  name.value = t.name;
  name.placeholder = "Name, e.g. Standup";
  name.setAttribute("aria-label", "Transform name");
  name.onchange = () => saveTransforms((list) => (list[i].name = name.value.trim()));

  const place = document.createElement("select");
  place.setAttribute("aria-label", `Where ${t.name || "the result"} goes in Notes`);
  for (const [value, label] of [["replace", "Replaces the text"], ["below", "Goes below it"]]) {
    place.append(new Option(label, value));
  }
  place.value = t.place ?? "replace";
  place.onchange = () => saveTransforms((list) => (list[i].place = place.value));

  const remove = document.createElement("button");
  remove.className = "x";
  remove.textContent = "×";
  remove.title = "Remove";
  remove.setAttribute("aria-label", `Remove transform ${t.name}`.trim());
  remove.onclick = () => saveTransforms((list) => list.splice(i, 1));

  top.append(name, place, transformShortcut(t, i), remove);

  const instruction = document.createElement("textarea");
  instruction.rows = 2;
  instruction.value = t.instruction;
  instruction.placeholder = "What to do with the text, e.g. Turn this into Yesterday / Today / Blockers bullets.";
  instruction.setAttribute("aria-label", `Instruction for ${t.name || "this transform"}`);
  instruction.onchange = () => saveTransforms((list) => (list[i].instruction = instruction.value.trim()));

  card.append(top, instruction);
  const error = state.shortcut_errors?.[`transform:${t.id}`];
  if (error) {
    const warn = document.createElement("small");
    warn.className = "warn-text";
    warn.textContent = `${shortcutLabel(t.shortcut ?? "", state.platform)} ${error}. Choose another.`;
    card.append(warn);
  }
  return card;
}

/** The shortcut picker, as on presets. */
function transformShortcut(t, i) {
  const wrap = document.createElement("span");
  wrap.className = "keypicker shortcut-pick";
  const main = document.createElement("button");
  main.type = "button";
  main.className = "kp-main";
  const cap = document.createElement("span");
  if (t.shortcut) {
    cap.className = "kbd";
    cap.textContent = shortcutLabel(t.shortcut, state.platform);
  } else {
    cap.className = "kp-hint none";
    cap.textContent = "No shortcut";
  }
  main.append(cap);
  main.setAttribute("aria-label", `Shortcut for ${t.name}: ${t.shortcut ? shortcutLabel(t.shortcut, state.platform) : "none"}. Click, then press the shortcut.`);
  main.onclick = () =>
    recordShortcut((s) => saveTransforms((list) => (list[i].shortcut = s)), renderTransforms, wrap, main);
  wrap.append(main);
  if (t.shortcut) {
    const clear = document.createElement("button");
    clear.type = "button";
    clear.className = "kp-more";
    clear.textContent = "×";
    clear.setAttribute("aria-label", `Remove the shortcut for ${t.name}`);
    clear.onclick = () => saveTransforms((list) => (list[i].shortcut = null));
    wrap.append(clear);
  }
  return wrap;
}

/** Applies `change` to the list in effect and saves all of it. */
function saveTransforms(change) {
  const s = collect();
  const list = structuredClone(state.transforms ?? []);
  change(list);
  // Half-filled cards are kept while editing; empty ones are dropped.
  s.transforms = list.filter((t) => t.name || t.instruction);
  state.transforms = s.transforms;
  save(s);
}

$("transform-add").addEventListener("click", () => {
  const id = `custom-${Date.now().toString(36)}`;
  state.transforms = [...(state.transforms ?? []), { id, name: "", instruction: "", place: "replace", shortcut: null }];
  renderTransforms();
  $("transforms").querySelector(".transform:last-child input")?.focus();
});

$("transform-reset").addEventListener("click", async () => {
  const ok = await askConfirm("Restore the built-in transforms?", {
    action: "Restore",
    detail: "Your own transforms and changes to the built-in ones are removed.",
  });
  if (!ok) return;
  const s = collect();
  s.transforms = null;
  save(s);
});
