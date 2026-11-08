"use strict";

// What you can say: the sections the Rust side builds from the settings
// (`help.rs`), redrawn whenever the window is shown again. Esc closes.

function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function render(sheet) {
  $("hp-sections").replaceChildren(
    ...sheet.sections.map((section) => {
      const block = el("section", "hp-section");
      block.append(el("h2", null, section.title));
      if (section.how) block.append(el("p", "hp-how", section.how));
      for (const item of section.items) {
        const row = el("div", "hp-row");
        const say = el("div", "hp-say");
        if (item.keys) say.append(el("span", "kbd", item.keys));
        say.append(...item.say.map((s) => el("q", null, s)));
        row.append(el("div", "hp-what", item.what), say);
        block.append(row);
      }
      if (section.actions && sheet.actions_off) {
        block.append(button("Turn on voice actions", () => call("help_open_actions"), "btn primary hp-turn-on"));
      }
      return block;
    }),
  );
  $("hp-sections").scrollTop = 0;
}

$("hp-close").addEventListener("click", () => call("help_close"));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    call("help_close");
  }
});

listen("help-changed", (event) => render(event.payload));
invoke("help_sheet")
  .then(render)
  .catch((e) => toast(String(e), true));
