"use strict";

// The window's menus.

/** Opens menu `name` ("more", "transcribe") or, with null, closes them. */
function setMenus(name) {
  hideItemMenu();
  for (const [key, btn, menu] of [["more", "nt-more", "nt-menu"], ["transcribe", "nt-transcribe", "nt-transcribe-menu"]]) {
    const open = key === name;
    $(menu).hidden = !open;
    $(btn).setAttribute("aria-expanded", String(open));
    if (open) $(menu).querySelector("button:not(:disabled)")?.focus();
  }
}

$("nt-more").addEventListener("click", (e) => {
  e.stopPropagation();
  setMenus($("nt-menu").hidden ? "more" : null);
});
$("nt-transcribe").addEventListener("click", (e) => {
  e.stopPropagation();
  setMenus($("nt-transcribe-menu").hidden ? "transcribe" : null);
});
document.addEventListener("click", () => {
  setMenus(null);
  hideItemMenu();
});

const ACTS = {
  rename: startRename,
  reveal,
  folder: changeFolder,
  delete: deleteNote,
  "edit-rewrites": editRewrites,
  "on-top": () => setOnTop(!onTop),
  "hide-from-agents": toggleHiddenFromAgents,
  transcribe: pickFile,
  "transcribe-link": askForLink,
  "transcribe-app": pickApp,
  "transcribe-mic": recordMicrophone,
  meeting: toggleMeeting,
  ask: () => openAsk({ scope: "all" }),
};
for (const menu of [$("nt-menu"), $("nt-transcribe-menu"), $("nt-start")]) {
  menu.addEventListener("click", (e) => {
    const act = e.target.closest("button")?.dataset.act;
    if (!act) return;
    setMenus(null);
    ACTS[act]();
  });
}
// Arrow keys move through an open menu.
for (const menu of [$("nt-menu"), $("nt-transcribe-menu"), $("nt-item-menu")]) {
  menu.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...menu.querySelectorAll("button:not(:disabled)")];
    const at = items.indexOf(document.activeElement);
    items[(at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
  });
}
