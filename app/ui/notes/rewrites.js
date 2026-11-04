"use strict";

// Rewriting a note with a transform, and reviewing what changed before keeping it.

/** Saved instructions, from Settings → Words. */
let transforms = [];

/** "Ctrl+Alt+Digit1" → "⌃⌥1" on a Mac, "Ctrl+Alt+1" elsewhere. */
function shortcutLabel(s) {
  const parts = s.split("+");
  const key = parts.pop().replace(/^Key/, "").replace(/^Digit/, "");
  if (!isMac) return [...parts, key].join("+");
  const symbol = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Cmd: "⌘" };
  return parts.map((m) => symbol[m] ?? m).join("") + key;
}

async function loadTransforms() {
  try {
    transforms = await invoke("notes_transforms") ?? [];
  } catch (e) {
    toast(String(e), true);
  }
  // The meeting summary has its own place, in a meeting's Summary tab.
  const chips = transforms.filter((t) => t.id !== "meeting-summary").map((t) => {
    const b = document.createElement("button");
    b.className = "nt-chip";
    b.textContent = t.name;
    b.title = t.shortcut ? `${t.name} (${shortcutLabel(t.shortcut)})` : t.name;
    b.addEventListener("click", () => runTransform(t.id));
    return b;
  });
  $("nt-rewrite-chips").replaceChildren(...chips);
  updateHeader();
}

/** The rewrite being looked at, or null: the text it's of (the selection
 *  or the whole note), the note as it was, and each try. */
let review = null;

function canRewrite() {
  return !$("nt-edit-wrap").hidden && !locked() && !review;
}

/** Starts a rewrite of the selection, or of the whole note: transform
 *  `id`, or (with none) only what `extra` asks for. */
async function runTransform(id, { extra = null } = {}) {
  if (!canRewrite()) {
    if (!review) toast("Open a note to rewrite it.");
    return;
  }
  showHistory(false);
  await saveNow();
  const sel = editor.selection;
  const whole = sel.from === sel.to;
  const text = editor.text;
  const source = whole ? text : text.slice(sel.from, sel.to);
  if (!source.trim()) return toast("There's no text to change.");
  review = {
    path: current.path,
    tab,
    before: text,
    from: whole ? 0 : sel.from,
    to: whole ? text.length : sel.to,
    whole,
    source,
    tries: [],
    shown: -1,
    busy: false,
  };
  showReview(true);
  if (id || extra) await rewrite(id, extra);
  else {
    renderReview();
    $("nt-review-extra").focus();
  }
}

/** One try: transform `id` with `extra` on the original text. */
async function rewrite(id, extra) {
  if (!review || review.busy) return;
  const t = transforms.find((x) => x.id === id);
  const name = t?.name ?? "Rewrite";
  const asked = review;
  asked.busy = { name };
  renderReview();
  let out;
  try {
    out = await invoke("notes_transform", { path: asked.path, id: id ?? "", text: asked.source, extra });
  } catch (e) {
    toast(String(e), true);
  }
  if (review !== asked) return;
  asked.busy = false;
  if (out?.unchanged) toast(`${name}: ${out.unchanged}`);
  if (!out || out.unchanged) {
    if (!asked.tries.length && id) return closeReview();
    return renderReview();
  }
  let label = extra ? (t ? `${name}, your way` : "Your way") : name;
  const same = asked.tries.filter((x) => x.label === label || x.label.startsWith(`${label} `)).length;
  if (same) label = `${label} ${same + 1}`;
  asked.tries.push({ label, text: out.text, place: t?.place ?? "replace", id: id ?? "", extra });
  asked.shown = asked.tries.length - 1;
  $("nt-review-extra").value = "";
  renderReview();
  $("nt-review-keep").focus();
}

function showReview(show) {
  $("nt-review").hidden = !show;
  $("nt-editor").hidden = show;
  editor.readOnly = show || locked();
  if (!show) {
    $("nt-review-extra").value = "";
    updateHeader();
  }
}

function closeReview() {
  if (!review) return;
  review = null;
  showReview(false);
  render({ reload: true });
}

function words(text) {
  return (text.match(/[\p{L}\p{N}]+/gu) ?? []).length;
}

function renderReview() {
  if (!review) return;
  const r = review;
  const shown = r.tries[r.shown];
  const name = r.busy ? `${r.busy.name}…` : shown?.label ?? "Rewrite";
  $("nt-review-name").textContent = name;
  $("nt-review-info").textContent = r.busy
    ? "Working on it"
    : !r.tries.length
      ? "Say how to change it, below."
      : shown
        ? shown.place === "below"
          ? "Goes below the text"
          : `${words(r.source).toLocaleString()} → ${words(shown.text).toLocaleString()} words`
        : "As it was";
  // Original, then each try.
  const seg = $("nt-review-versions");
  seg.hidden = !r.tries.length;
  const buttons = [{ label: "Original", i: -1 }, ...r.tries.map((x, i) => ({ label: x.label, i }))].map(({ label, i }) => {
    const b = document.createElement("button");
    b.textContent = label;
    b.setAttribute("aria-pressed", String(i === r.shown));
    b.disabled = !!r.busy;
    b.addEventListener("click", () => {
      r.shown = i;
      renderReview();
    });
    return b;
  });
  seg.replaceChildren(...buttons);
  const showDiff = $("nt-review-diff").checked;
  $("nt-review-diff-wrap").hidden = !shown || shown.place === "below";
  $("nt-review-keep").disabled = !shown || !!r.busy;
  $("nt-review-keep").textContent = shown?.place === "below" ? "Add it below" : "Keep this";
  $("nt-review-retry").disabled = !!r.busy;
  const out = $("nt-review-text");
  out.classList.toggle("nt-working", !!r.busy && !shown);
  if (!shown) {
    out.textContent = r.source;
  } else if (shown.place === "below" || !showDiff) {
    out.textContent = shown.text;
  } else {
    out.replaceChildren(...diffNodes(r.source, shown.text));
  }
}

/** Keeps the try shown: in place of the text (⌘Z undoes it), or below it. */
function keepRewrite() {
  const r = review;
  const shown = r?.tries[r.shown];
  if (!shown || r.busy) return;
  const changed = current.path !== r.path || tab !== r.tab || editor.text !== r.before;
  closeReview();
  if (changed) {
    invoke("copy_text", { text: shown.text }).catch(() => {});
    return toast("The note changed meanwhile, so the rewrite was copied instead.");
  }
  editor.focus();
  if (shown.place === "below") {
    const before = editor.text.slice(0, r.to);
    const insert = `${before.endsWith("\n") ? "" : "\n"}\n---\n\n${shown.text}`;
    editor.replace(r.to, r.to, insert, { select: false });
  } else {
    editor.replace(r.from, r.to, shown.text, { select: !r.whole });
  }
  toast(`Kept. ${isMac ? "⌘Z" : "Ctrl+Z"} undoes it; the note before is in Versions.`);
}

$("nt-review-keep").addEventListener("click", keepRewrite);
$("nt-review-back").addEventListener("click", () => {
  closeReview();
  focusNote();
});
$("nt-review-diff").addEventListener("change", renderReview);
$("nt-review-again").addEventListener("submit", (e) => {
  e.preventDefault();
  if (!review) return;
  const extra = $("nt-review-extra").value.trim() || null;
  const last = review.tries[review.shown] ?? review.tries[review.tries.length - 1];
  const id = last?.id ?? "";
  if (!id && !extra) return $("nt-review-extra").focus();
  rewrite(id, extra ?? last?.extra ?? null);
});
$("nt-sayhow").addEventListener("click", () => runTransform("", {}));

// A transform's shortcut pressed while Notes is in front.
listen("notes-run-transform", (event) => runTransform(event.payload));

/** Words, spaces and marks, for comparing two texts. */
function tokens(text) {
  return text.match(/\s+|[\p{L}\p{N}_'’]+|[^\s\p{L}\p{N}_]/gu) ?? [];
}

/** The edits that turn token list `a` into `b`, as [op, token] with op
 *  "=", "-" or "+" (Myers' diff). Null when they differ too much to show
 *  usefully. */
function diffTokens(a, b, limit = 3000) {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const A = a.slice(start, endA);
  const B = b.slice(start, endB);
  const n = A.length;
  const m = B.length;
  const max = n + m;
  const off = max + 1;
  const v = new Int32Array(2 * max + 3);
  const trace = [];
  let found = -1;
  outer: for (let d = 0; d <= max; d++) {
    if (d > limit) return null;
    trace.push(v.slice(off - d - 1, off + d + 2));
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && A[x] === B[y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break outer;
      }
    }
  }
  const ops = [];
  let x = n;
  let y = m;
  for (let d = found; d > 0; d--) {
    const vv = trace[d];
    const get = (k) => vv[k + d + 1];
    const k = x - y;
    const prevK = k === -d || (k !== d && get(k - 1) < get(k + 1)) ? k + 1 : k - 1;
    const prevX = get(prevK);
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      ops.push(["=", A[x - 1]]);
      x--;
      y--;
    }
    if (x === prevX) ops.push(["+", B[prevY]]);
    else ops.push(["-", A[prevX]]);
    x = prevX;
    y = prevY;
  }
  while (x > 0 && y > 0) {
    ops.push(["=", A[x - 1]]);
    x--;
    y--;
  }
  ops.reverse();
  return [...a.slice(0, start).map((t) => ["=", t]), ...ops, ...a.slice(endA).map((t) => ["=", t])];
}

/** `after` with what changed from `before` marked: removed words struck
 *  through, added ones highlighted. */
function diffNodes(before, after) {
  const ops = diffTokens(tokens(before), tokens(after));
  if (!ops) return [document.createTextNode(after)];
  // Runs of one kind; a space between two changes joins them.
  const runs = [];
  for (const [op, t] of ops) {
    const last = runs[runs.length - 1];
    if (last && last.op === op) last.text += t;
    else runs.push({ op, text: t });
  }
  for (let i = 1; i < runs.length - 1; i++) {
    const r = runs[i];
    if (r.op === "=" && /^[ \t]+$/.test(r.text) && runs[i - 1].op !== "=" && runs[i + 1].op !== "=") {
      r.op = "~";
    }
  }
  const nodes = [];
  let removed = "";
  let added = "";
  const flush = () => {
    if (removed) {
      const del = document.createElement("del");
      del.textContent = removed;
      nodes.push(del);
    }
    if (added) {
      const ins = document.createElement("ins");
      ins.textContent = added;
      nodes.push(ins);
    }
    removed = "";
    added = "";
  };
  for (const r of runs) {
    if (r.op === "-") removed += r.text;
    else if (r.op === "+") added += r.text;
    else if (r.op === "~") {
      removed += r.text;
      added += r.text;
    } else {
      flush();
      nodes.push(document.createTextNode(r.text));
    }
  }
  flush();
  return nodes;
}
