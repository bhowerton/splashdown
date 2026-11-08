"use strict";

// Permissions tab: Accessibility and Microphone (rows drawn by common.js),
// the "all set" banner and sidebar dot, and polling for changes.

function permissionsDone(p = state.permissions) {
  return p.accessibility && p.microphone === "granted";
}

function renderPermissions() {
  renderPermissionRows(state.permissions, "perm-", "step");
  // Windows asks only for the microphone (like the setup guide).
  const windows = state.platform !== "macos";
  $("perm-ax-row").hidden = windows;
  $("perm-mic-step").textContent = windows ? "1" : "2";
  $("perm-secure-note").hidden = windows;
  $("perm-lead").textContent = windows
    ? "What Windows needs to let Splashdown hear you."
    : "What macOS needs to let Splashdown hear the key and type for you.";
  $("perm-intro").textContent = windows
    ? "One quick step. It updates here by itself as soon as you allow it."
    : "Two quick steps. Each one updates here by itself as soon as you allow it.";
  const done = permissionsDone();
  $("perm-done").hidden = !done;
  $("perm-intro").hidden = done;
  $("perm-key").textContent = dictationKeyLabel(state);
  $("perm-badge").hidden = done;
}

/** Permissions change in System Settings; keep the indicators current. */
async function pollPermissions() {
  try {
    const fresh = await invoke("get_state");
    const wasDone = permissionsDone();
    const hadAx = state.permissions.accessibility;
    state.permissions = fresh.permissions;
    renderPermissions();
    if (!hadAx && fresh.permissions.accessibility) toast("Accessibility allowed");
    if (!wasDone && permissionsDone()) toast("All set — try dictating");
  } catch {
    /* window closing */
  }
}
