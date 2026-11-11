"use strict";

// Search across every tab of Settings: the field at the top of the sidebar
// lists the settings whose name, description, group or tab has the words
// typed, and choosing one goes to it. The index is the page itself, read
// when a search is made, so a setting added to a tab is found without
// anything being added here.

/** Other words for the same thing, so that the word someone thinks of finds
 *  the one the page uses. A word typed matches when it, or any word in its
 *  row here, is in the setting's text. */
const SEARCH_SYNONYMS = [
  ["shortcut", "hotkey", "key"],
  ["mic", "microphone"],
  ["hook", "webhook", "automation", "when something happens"],
  ["pill", "mini bar", "button on screen"],
  ["update", "version"],
  ["speaker", "voice", "people apart", "who said"],
  ["summary", "summarize"],
  ["location", "folder", "where"],
  ["paste", "clipboard"],
  ["llm", "model", "ai"],
  ["stt", "transcription", "speech to text"],
  ["tts", "read aloud", "text to speech"],
  ["cost", "budget", "spending", "limit"],
  ["startup", "login", "launch"],
  ["beep", "sound", "volume"],
  ["dictionary", "vocabulary", "names and terms"],
  ["command", "action"],
  ["delete", "forget", "clear"],
  ["recording", "audio"],
];

/** The tab that was open when a search began, to go back to. */
let searchFrom = null;

/** A node's text without that of its `<small>` (a row's name without its
 *  description), on one line. */
function ownText(node, skip = "small") {
  const copy = node.cloneNode(true);
  copy.querySelectorAll(skip).forEach((n) => n.remove());
  return copy.textContent.replace(/\s+/g, " ").trim();
}

/** Everything that can be found: each tab, each group in it, and each
 *  setting, skipping what this computer doesn't show. */
function searchIndex() {
  const found = [];
  for (const tab of document.querySelectorAll(".tab:not(#tab-search)")) {
    const id = tab.id.slice(4);
    const link = document.querySelector(`.side a[data-tab="${id}"]`);
    if (!link) continue;
    const tabName = ownText(link, ".tile, .dot-warn");
    found.push({ tab: id, el: tab.querySelector("h1"), name: tabName, where: "", about: tab.querySelector(".page-head p")?.textContent ?? "" });
    const shown = (el) => {
      for (let n = el; n && n !== tab; n = n.parentElement) if (n.hidden) return false;
      return true;
    };
    for (const group of tab.querySelectorAll(".gh")) {
      if (shown(group)) found.push({ tab: id, el: group, name: ownText(group), where: tabName, about: "" });
    }
    for (const label of tab.querySelectorAll(".row .lbl")) {
      const row = label.closest(".row");
      const name = ownText(label);
      if (!name || !shown(row)) continue;
      const group = row.closest(".group")?.querySelector(".gh");
      const groupName = group ? ownText(group) : "";
      found.push({
        tab: id,
        el: row,
        name,
        where: groupName ? `${tabName} › ${groupName}` : tabName,
        about: label.querySelector("small")?.textContent.replace(/\s+/g, " ").trim() ?? "",
      });
    }
  }
  return found;
}

/** How well `item` answers the words typed: 0 when one of them is nowhere
 *  in it; higher when they're in its name than in where it is or what it
 *  says. */
function searchScore(item, words) {
  const name = item.name.toLowerCase();
  const where = item.where.toLowerCase();
  const about = item.about.toLowerCase();
  let score = 0;
  for (const typed of words) {
    // "calendars" finds "calendar".
    const word = typed.length > 3 ? typed.replace(/s$/, "") : typed;
    const same = SEARCH_SYNONYMS.find((row) => row.includes(word) || row.includes(typed)) ?? [];
    // Another word for it has to be there as a word of its own ("key", not
    // the start of "Keychain"), give or take a plural.
    const any = (text, list) =>
      list.some((w) => new RegExp(`(^|[^a-z])${w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}s?([^a-z]|$)`).test(text));
    if (name.includes(word)) score += 8;
    else if (any(name, same)) score += 5;
    else if (where.includes(word) || any(where, same)) score += 3;
    else if (about.includes(word)) score += 2;
    // Another word for it counts in a name or a place, not in a
    // description: half of them say "key" somewhere.
    else return 0;
  }
  // A tab or a group is the place for all of it: ahead of its own rows.
  if (item.el.matches("h1, .gh")) score += 1;
  return score;
}

/** Goes to a setting: its tab, with the row in view and marked a moment. */
function goToSetting(item) {
  $("set-search").value = "";
  searchFrom = null;
  try {
    history.replaceState(null, "", `#${item.tab}`);
  } catch {
    /* not every page origin allows it; the tab still switches */
  }
  showTab(item.tab);
  const el = item.el;
  if (!el?.isConnected) return;
  for (let n = el.parentElement; n; n = n.parentElement) if (n.tagName === "DETAILS") n.open = true;
  el.scrollIntoView({ block: "center" });
  el.classList.remove("found");
  // Restart the mark if the same one is chosen twice.
  void el.offsetWidth;
  el.classList.add("found");
  setTimeout(() => el.classList.remove("found"), 2200);
  const control = el.querySelector("input, select, button, textarea, [tabindex]");
  (control ?? el).focus?.({ preventScroll: true });
}

/** Shows what the words in the field find, or the tab that was open when
 *  the field is emptied. */
function runSearch() {
  const typed = $("set-search").value.trim().toLowerCase();
  const section = $("tab-search");
  if (!typed) {
    if (searchFrom) {
      const back = searchFrom;
      searchFrom = null;
      showTab(back);
    }
    return;
  }
  if (!searchFrom) searchFrom = document.querySelector(".side a.on")?.dataset.tab ?? "general";
  const words = typed.split(/\s+/);
  const results = searchIndex()
    .map((item) => ({ item, score: searchScore(item, words) }))
    .filter((r) => r.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 40)
    .map((r) => r.item);

  document.querySelectorAll(".tab").forEach((s) => (s.hidden = s !== section));
  document.querySelectorAll(".side a[data-tab]").forEach((a) => {
    a.classList.remove("on");
    a.removeAttribute("aria-current");
  });
  $("search-count").textContent = results.length
    ? `${results.length === 40 ? "The first 40" : results.length} for “${$("set-search").value.trim()}”`
    : `Nothing for “${$("set-search").value.trim()}”. Try another word: settings are found by their names and descriptions.`;
  const list = $("search-results");
  list.replaceChildren(
    ...results.map((item) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "row result";
      const text = document.createElement("span");
      text.className = "lbl";
      text.append(item.name);
      const small = document.createElement("small");
      small.textContent = item.about.length > 150 ? `${item.about.slice(0, 150).trimEnd()}…` : item.about;
      if (item.about) text.append(small);
      const where = document.createElement("span");
      where.className = "result-where";
      where.textContent = item.where;
      b.append(text, where);
      b.addEventListener("click", () => goToSetting(item));
      return b;
    }),
  );
  list.hidden = !results.length;
}

$("set-search").addEventListener("input", runSearch);
$("set-search").addEventListener("keydown", (e) => {
  if (e.key === "Escape" && $("set-search").value) {
    e.preventDefault();
    $("set-search").value = "";
    runSearch();
  } else if (e.key === "Enter") {
    // The best match.
    $("search-results").querySelector("button")?.click();
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    $("search-results").querySelector("button")?.focus();
  }
});
// Up and down through what was found, and back to the field from the top.
$("search-results").addEventListener("keydown", (e) => {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  e.preventDefault();
  const next = e.key === "ArrowDown" ? e.target.nextElementSibling : e.target.previousElementSibling;
  (next ?? (e.key === "ArrowUp" ? $("set-search") : e.target)).focus();
});
// ⌘F / Ctrl+F is the search field.
document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f") {
    e.preventDefault();
    $("set-search").focus();
    $("set-search").select();
  }
});
