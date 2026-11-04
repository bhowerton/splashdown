"use strict";

// Asking the notes a question (⌘J), with the answer's sources.

const askState = {
  scope: "all",
  /** Questions and answers: { q, a, done, error, sources, asked, recent }. */
  log: [],
  next: 1,
};

function openAsk({ scope = null, question = null, recent = false } = {}) {
  $("nt-askpanel").hidden = false;
  $("nt-ask").setAttribute("aria-pressed", "true");
  if (scope) setAskScope(scope);
  else if (askState.log.length === 0) setAskScope(current.path ? "note" : "all");
  renderAsk();
  if (question) ask(question, { recent });
  else $("nt-ask-input").focus();
}

function closeAsk() {
  $("nt-askpanel").hidden = true;
  $("nt-ask").setAttribute("aria-pressed", "false");
  focusNote();
}

function setAskScope(scope) {
  if (scope === askState.scope) return;
  askState.scope = scope;
  // A new conversation: the earlier one was about something else.
  askState.log = [];
  renderAsk();
}

async function ask(question, { recent = liveHere() } = {}) {
  question = question.trim();
  if (!question) return;
  let path = null;
  if (askState.scope === "note") {
    await saveNow();
    path = current.path;
    if (!path) setAskScope("all");
  }
  const earlier = askState.log
    .filter((e) => e.done && !e.error && e.a)
    .slice(-ASK_EARLIER)
    .map((e) => [e.q, stripCites(e.a)]);
  const entry = { q: question, a: "", done: false, error: null, sources: [], asked: askState.next++, recent };
  askState.log.push(entry);
  renderAsk();
  try {
    await invoke("notes_ask", { question, path, recent, earlier, asked: entry.asked });
  } catch (e) {
    entry.done = true;
    entry.error = String(e);
    renderAsk();
  }
}

function stripCites(text) {
  return text.replace(/\s?\[\d+\]/g, "");
}

listen("notes-ask", (event) => askUpdated(event.payload));

function askUpdated(u) {
  const entry = askState.log.find((e) => e.asked === u.asked);
  if (!entry) return;
  entry.a = u.answer;
  entry.done = u.done;
  entry.error = u.error;
  entry.sources = u.sources;
  renderAsk();
}

/** Text with **bold** and [n] citations as nodes. */
function inline(text, sources) {
  const out = [];
  const re = /\*\*([^*]+)\*\*|\[(\d+)\]/g;
  let at = 0;
  for (let m; (m = re.exec(text)); ) {
    if (m.index > at) out.push(document.createTextNode(text.slice(at, m.index)));
    if (m[1] != null) {
      const b = document.createElement("b");
      b.textContent = m[1];
      out.push(b);
    } else {
      const source = sources[Number(m[2]) - 1];
      if (source) {
        const cite = document.createElement("button");
        cite.className = "nt-cite";
        cite.textContent = m[2];
        cite.title = source.title;
        cite.setAttribute("aria-label", `Source ${m[2]}: ${source.title}`);
        cite.addEventListener("click", () => openNote(source.path));
        out.push(cite);
      } else {
        out.push(document.createTextNode(m[0]));
      }
    }
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push(document.createTextNode(text.slice(at)));
  return out;
}

/** An answer: paragraphs and lists, with its citations as buttons. */
function answerNodes(text, sources) {
  const nodes = [];
  let list = null;
  for (const line of text.split("\n")) {
    const item = /^\s*(?:[-*•]|\d+[.)])\s+(.*)$/.exec(line);
    if (item) {
      if (!list) {
        list = document.createElement("ul");
        nodes.push(list);
      }
      const li = document.createElement("li");
      li.append(...inline(item[1], sources));
      list.append(li);
    } else if (line.trim()) {
      list = null;
      const p = document.createElement("p");
      p.append(...inline(line.replace(/^#+\s*/, ""), sources));
      nodes.push(p);
    } else {
      list = null;
    }
  }
  return nodes;
}

function renderAsk() {
  for (const b of document.querySelectorAll("#nt-askpanel [data-scope]")) {
    b.setAttribute("aria-pressed", String(b.dataset.scope === askState.scope));
    if (b.dataset.scope === "note") b.disabled = !current.path;
  }
  $("nt-ask-input").placeholder = askState.log.length
    ? "Ask a follow-up"
    : askState.scope === "note"
      ? `Ask about “${titleOf(body) || "this note"}”`
      : "Ask your notes";
  const rows = [];
  if (!askState.log.length) {
    const hint = document.createElement("p");
    hint.className = "nt-hint";
    hint.textContent = askState.scope === "note"
      ? "Ask about this note: what was decided, who's doing what, or anything else in it."
      : "Ask about anything in your notes. Answers say which notes they came from.";
    rows.push(hint);
  }
  for (const e of askState.log) {
    const q = document.createElement("div");
    q.className = "nt-q";
    q.textContent = e.q;
    rows.push(q);
    const a = document.createElement("div");
    a.className = `nt-a${e.error ? " nt-error" : ""}`;
    if (e.error) a.textContent = e.error;
    else if (!e.a) a.replaceChildren(Object.assign(document.createElement("span"), { className: "nt-pending", textContent: "Thinking…" }));
    else a.replaceChildren(...answerNodes(e.a, e.sources));
    rows.push(a);
    if (e.done && !e.error && e.a) {
      const cited = new Set([...e.a.matchAll(/\[(\d+)\]/g)].map((m) => Number(m[1])));
      const sources = e.sources.map((s, i) => ({ ...s, n: i + 1 })).filter((s) => cited.has(s.n) || e.sources.length === 1);
      if (sources.length) {
        const box = document.createElement("div");
        box.className = "nt-sources";
        for (const s of sources) {
          const b = document.createElement("button");
          b.className = "nt-source";
          const n = document.createElement("span");
          n.className = "nt-cite";
          n.textContent = String(s.n);
          const title = document.createElement("span");
          title.textContent = s.title || s.path;
          b.append(n, title);
          b.title = `Open ${s.title || s.path}`;
          b.addEventListener("click", () => openNote(s.path));
          box.append(b);
        }
        rows.push(box);
      }
      const acts = document.createElement("div");
      acts.className = "nt-a-acts";
      acts.append(
        button("Copy", () => invoke("copy_text", { text: stripCites(e.a) }).then(() => toast("Copied")).catch((err) => toast(String(err), true))),
      );
      const add = button("Add to this note", () => addToNote(stripCites(e.a)));
      add.disabled = $("nt-edit-wrap").hidden || locked() || !!review;
      acts.append(add);
      rows.push(acts);
    }
  }
  const log = $("nt-ask-log");
  log.replaceChildren(...rows);
  log.scrollTop = log.scrollHeight;
}

function addToNote(text) {
  if ($("nt-edit-wrap").hidden || locked() || review) return;
  const end = editor.text.length;
  const before = editor.text;
  const gap = !before.trim() ? "" : before.endsWith("\n\n") ? "" : before.endsWith("\n") ? "\n" : "\n\n";
  editor.focus();
  editor.replace(end, end, `${gap}${text.trim()}\n`, { select: false });
  toast("Added to the end of the note");
}

$("nt-ask").addEventListener("click", () => ($("nt-askpanel").hidden ? openAsk() : closeAsk()));
$("nt-ask-close").addEventListener("click", closeAsk);
$("nt-ask-clear").addEventListener("click", () => {
  askState.log = [];
  renderAsk();
  $("nt-ask-input").focus();
});
for (const b of document.querySelectorAll("#nt-askpanel [data-scope]")) {
  b.addEventListener("click", () => setAskScope(b.dataset.scope));
}
$("nt-ask-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const input = $("nt-ask-input");
  const q = input.value;
  input.value = "";
  ask(q);
});
$("nt-missed").addEventListener("click", () =>
  openAsk({ scope: "note", recent: true, question: "What was said in the last five minutes? Just the main points, briefly." }),
);
$("nt-live-ask").addEventListener("click", () => openAsk({ scope: "note" }));
