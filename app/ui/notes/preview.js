"use strict";

// Sample backend for previewing the window as a plain file:// page (only when window.__TAURI__ is absent).

/** Sample backend for opening this page as a plain file:// page (see
 *  invoke in common.js). */
async function mockInvoke(cmd, args) {
  const hour = 3600 * 1000;
  const meetingBody = "# Weekly product sync · Oct 28, 9:00 AM\n\nRecorded on this computer · transcribed on this computer.\n\n## My notes\n\n- Comparison table → 3 rows\n- Annual: 2 months free?\n\n## Summary\n\n### Overview\nThe team agreed to shorten the pricing page's comparison table and to look at an annual plan.\n\n### Decisions\n- Cut the comparison table to three rows; the rest moves to a details page.\n\n### Action items\n- [ ] Model per-seat pricing with and without the annual discount, by Thursday (Them)\n- [x] Reply to Dana with the plan (You)\n\n## Transcript\n\n**[16:58] Them:** So for the pricing page I think the comparison table is doing more harm than good.\n\n**[17:20] You:** Agreed. Dana said the same in the beta notes.\n\n**[17:41] Them:** And the annual plan. If we do two months free, does that change the team price per seat?\n\n**[18:15] You:** It shouldn't. Can you model both and bring it Thursday?\n\n*Ended at 34:10.*\n\n[Recording](Attachments/2026-10-28%20Weekly%20product%20sync.wav)\n";
  const list = [
    { path: "Work/Launch/2026-10-31 Launch post.md", title: "Launch post", text: "Launch post\n- lead with the hold-to-talk demo #marketing\n- mention it's **free** and open source\n- [ ] ask Dana about the table", modified: Date.now() - 0.2 * hour, pinned: true, folder: "Work/Launch", source: "", tags: ["launch", "marketing"] },
    { path: "Meetings/2026-10-28 Weekly product sync.md", title: "Weekly product sync · Oct 28, 9:00 AM", text: meetingBody, modified: Date.now() - 1 * hour, pinned: false, folder: "Meetings", source: "meeting" },
    { path: "2026-10-31 Call the dentist.md", title: "Call the dentist", text: "Call the dentist\nabout moving Thursday's appointment.", modified: Date.now() - 2 * hour, pinned: false, folder: "", source: "capture" },
    { path: "2026-10-30 Writing for the ear.md", title: "Writing for the ear", text: "# Writing for the ear\n\nTranscribed from <https://www.youtube.com/watch?v=x> (12:04) on this computer.\n\n**[0:00]** Welcome back.", modified: Date.now() - 30 * hour, pinned: false, folder: "", source: "file" },
    { path: "Work/Standup.md", title: "Standup", text: "Standup\nWindows build compiles; testing Monday. #work/acme", modified: Date.now() - 80 * hour, pinned: false, folder: "Work", source: "", tags: ["work/acme"] },
    { path: "Personal/Recipes/Bread.md", title: "Bread", text: "Bread\nflour, water, salt", modified: Date.now() - 300 * hour, pinned: false, folder: "Personal/Recipes", source: "", tags: [] },
  ];
  const open = (path) => {
    const n = list.find((x) => x.path === path) ?? list[0];
    return { path: n.path, body: n.text, modified: n.modified, source: n.source, front_tags: n.path.includes("Launch") ? ["launch"] : [] };
  };
  switch (cmd) {
    case "notes_load":
      return {
        folder: { path: "/Users/you/Documents/Splashdown Notes", label: "Splashdown Notes", synced: null, problem: null },
        notes: list,
        open: open(list[0].path),
        target: "Mail",
      };
    case "notes_list":
      return list;
    case "notes_daily":
      return { path: "Daily/2026-11-01.md", body: "# 2026-11-01\n\n", modified: Date.now(), source: "", front_tags: [] };
    case "notes_folders":
      return ["Meetings", "Personal", "Personal/Recipes", "Work", "Work/Launch", "Archive"];
    case "notes_open":
      return open(args.path);
    case "notes_transcribe_again_models":
      return [{ id: null, label: "The speech model in Settings" }, { id: "parakeet-v3", label: "Parakeet v3 · 25 European languages" }, { id: "large-v3-turbo", label: "Whisper large-v3-turbo · about 100 languages" }];
    case "notes_save":
      return { note: { path: args.path ?? "2026-11-01 New.md", body: args.text, modified: Date.now(), source: "" }, conflict: false };
    case "notes_versions":
      return [{ at: Date.now() - hour, preview: "Launch post\n- lead with the demo" }];
    case "notes_transforms":
      return [
        { id: "clean-up", name: "Clean up", place: "replace", shortcut: null },
        { id: "summary", name: "Summary", place: "below", shortcut: null },
        { id: "todos", name: "To-dos", place: "below", shortcut: "Ctrl+Alt+Digit1" },
        { id: "meeting-summary", name: "Meeting summary", place: "below", shortcut: null },
        { id: "email", name: "Email", place: "replace", shortcut: null },
      ];
    case "notes_transform":
      await new Promise((r) => setTimeout(r, 300));
      return { text: args.text.replace("lead with", "Lead with").replace("mention it's", "Say it's").replace(/- \[ \] ask Dana about the table/, "- [ ] Ask Dana whether the table helps"), unchanged: null };
    case "audio_apps":
      return [
        { id: "com.google.Chrome", name: "Google Chrome", playing: true, sources: [221, 222] },
        { id: "com.spotify.client", name: "Spotify", playing: true, sources: [230] },
        { id: "us.zoom.xos", name: "Zoom", playing: false, sources: [259] },
      ];
    case "app_info":
      return [];
    case "notes_ask":
      setTimeout(() => askUpdated({ asked: args.asked, answer: "Lead with the hold-to-talk demo [1], and say it's free [1]. The meeting agreed to cut the table [2].", done: true, error: null, sources: [{ title: "Launch post", path: list[0].path }, { title: "Weekly product sync", path: list[1].path }] }), 200);
      return null;
    default:
      return null;
  }
}
