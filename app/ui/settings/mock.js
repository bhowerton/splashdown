"use strict";

// Sample backend for previewing the Settings window as a plain file:// page.
// Only called when window.__TAURI__ is absent (see invoke in common.js).
// Keep it in step with the real commands: the state below mirrors
// `Snapshot` in app/src-tauri/src/snapshot.rs, and a command the page calls
// that isn't answered here logs a warning and returns null.

/** Sample data so the page can be designed in a normal browser. */
async function mockInvoke(cmd, args) {
  await new Promise((r) => setTimeout(r, 250));
  const base = {
    settings: {
      speech: { provider: "auto", model: null },
      ai: { provider: "auto", model: null, base_url: null, fallback_models: null },
      general: { user_name: "Alex", language: null, vocabulary: ["Tauri", "OpenRouter", "Priya"], learned_words: ["Priya"], live_preview: null, keep_history: null, check_for_updates: null },
      hotkeys: { dictation: null, instruction: null },
      presets: null,
      snippets: [{ trigger: "my address", text: "1 Infinite Loop, Cupertino" }],
      replacements: [{ find: "teh", replace: "the", regex: false }, { find: "gonna", replace: "going to", regex: false }],
      notes: {
        folder: null, capture_to: "inbox", daily_folder: null, daily_format: null, keep_audio: false,
        agents_read: false, agents_write: false, agents_meetings: null, agents_history: false, shortcuts_read: null,
        meeting_folder: null, meeting_summary: null, keep_meeting_audio: false, stop_when_quiet: null,
        meeting_pill: null, offer_on_calls: null, ask_uses_notes: null, meeting_reminded: false, keep_on_top: false,
        meeting_calendar: false, offer_on_calendar: false, meeting_speakers: false, remember_speakers: false, ask_by_meaning: false,
        watched: [{ path: "/Users/alex/Movies/Zoom", into: "Meetings", subtitles: false }],
      },
      actions: { enabled: true, off: ["run_shortcut"], search_engine: "google", confirm_events: false, trusted_shortcuts: ["Log water"] },
    },
    keys: [
      { name: "openai", source: null, hint: null },
      { name: "openrouter", source: "environment", hint: "9f2c" },
      { name: "anthropic", source: null, hint: null },
      { name: "custom", source: null, hint: null },
    ],
    permissions: { accessibility: true, microphone: "granted" },
    speech_models: [
      { id: "nemotron", mb: 495, downloaded: true, label: "Nemotron 3.5 · 32 languages · text as you speak", license: "OpenMDW-1.1", english_only: false },
      { id: "parakeet-v3", mb: 485, downloaded: false, label: "Parakeet v3 · 25 European languages", license: "CC-BY-4.0 (NVIDIA)", english_only: false },
      { id: "large-v3-turbo", mb: 619, downloaded: false, label: "Whisper large-v3-turbo · about 100 languages", license: "MIT (OpenAI)", english_only: false },
    ],
    spending: {
      month: { cost_usd: 0.42, requests: 1284, unpriced: 0, by_provider: { "openai/gpt-4o-mini-transcribe": 0.39, "openrouter/google/gemini-2.5-flash-lite": 0.03 } },
      today: { cost_usd: 0.031, requests: 96, unpriced: 0, by_provider: {} },
      state: "fine",
      prices_as_of: "2026-10-29",
    },
    local_models: [
      { id: "qwen3-4b", mb: 2500, ram_mb: 3000, downloaded: false, label: "Qwen3 4B · best quality" },
      { id: "qwen3-4b-compact", mb: 1670, ram_mb: 2300, downloaded: false, label: "Qwen3 4B compact · for about 8 GB of memory" },
      { id: "qwen3-1.7b", mb: 1100, ram_mb: 1700, downloaded: false, label: "Qwen3 1.7B · smallest" },
    ],
    key_options: [
      { value: "right-alt", label: "Right Option" },
      { value: "right-meta", label: "Right Command" },
      { value: "right-control", label: "Right Ctrl" },
      { value: "right-shift", label: "Right Shift" },
      { value: "left-alt", label: "Left Option" },
      { value: "left-control", label: "Left Ctrl" },
      { value: "fn", label: "Fn" },
    ],
    resolved_speech: { Ok: "OpenRouter · google/gemini-3.1-flash-lite" },
    resolved_ai: { Ok: "OpenRouter · google/gemini-2.5-flash-lite" },
    speech_destination: "cloud",
    ai_destination: "cloud",
    engine_error: null,
    resolved_instructions: null,
    can_route_instructions: true,
    downloads: [],
    speech_loading: false,
    apple: { speech: "ready", speech_detail: "", language_model: true, language_model_detail: "" },
    transforms: [
      { id: "summary", name: "Summary", instruction: "Summarize this in a few short bullet points.", place: "below", shortcut: null },
      { id: "todos", name: "To-dos", instruction: "List the tasks in this as - [ ] items.", place: "below", shortcut: "Ctrl+Alt+Digit1" },
      { id: "email", name: "Email", instruction: "Turn this into a short email.", place: "replace", shortcut: null },
    ],
    presets: [
      { id: "default", name: "Default", instructions: "", ai: true, insert: "paste", rules: [] },
      { id: "email", name: "Email", instructions: "Use short paragraphs. Only add a greeting or sign-off if I say one or ask for it.", ai: true, insert: "paste", rules: [{ app: "com.apple.mail", url: null, title: null }, { app: null, url: "mail.google.com", title: null }] },
      { id: "texting", name: "Texting", instructions: "", tone: "casual", ai: true, insert: "paste", rules: [{ app: "com.apple.MobileSMS", url: null, title: null }, { app: null, url: "web.whatsapp.com", title: null }] },
      { id: "chat", name: "Work chat", instructions: "No greeting or sign-off. Short paragraphs.", tone: "casual", auto_send: true, shortcut: "Ctrl+Alt+KeyC", ai: true, insert: "paste", rules: [{ app: "com.tinyspeck.slackmacgap", url: null, title: null }, { app: "slack.exe", url: null, title: null }] },
      { id: "code", name: "Code & Terminal", instructions: "", ai: false, insert: "paste", rules: [{ app: "com.apple.Terminal", url: null, title: null }] },
    ],
    preset_ai_options: [{ value: "apple", label: "Apple on-device" }, { value: "local", label: "Built-in local model" }, { value: "openrouter", label: "OpenRouter" }],
    shortcut_errors: {},
    defaults: {
      openai_stt: "gpt-4o-mini-transcribe",
      openrouter_stt: "google/gemini-3.1-flash-lite",
      local_speech: "nemotron",
      openai_ai: "gpt-4.1-mini",
      openrouter_ai: "google/gemini-2.5-flash-lite",
      anthropic_ai: "claude-haiku-5-5",
      sound_volume: 0.12,
    },
    platform: "macos",
    version: "0.7.0",
    config_path: "~/Library/Application Support/Splashdown/config.toml",
    update: { available: "0.2.0", notes: "Faster local speech-to-text.\nFixes pasting into Slack.", checking: false, installing: false, progress: null, last_checked: Date.now() - 3600000, error: null },
  };
  mockInvoke.state ??= base;
  switch (cmd) {
    case "get_state":
      return mockInvoke.state;
    case "save_settings":
      mockInvoke.state = { ...mockInvoke.state, settings: args.settings, presets: args.settings.presets ?? mockInvoke.state.presets };
      return { state: mockInvoke.state, engine_error: null };
    case "list_microphones":
      return { devices: [{ name: "MacBook Pro Microphone", bluetooth: false, built_in: true, is_default: true }, { name: "AirPods Pro", bluetooth: true, built_in: false, is_default: false }], automatic: "MacBook Pro Microphone" };
    case "reading_natural":
      return { downloaded: false, mb: 68 };
    case "reading_voices":
      return [
        { id: "natural:Harper", name: "Harper", language: "en-US", quality: "Natural", about: "Modern, confident and friendly." },
        { id: "natural:Maeve", name: "Maeve", language: "en-GB", quality: "Natural", about: "Sparky and attentive." },
        { id: "com.apple.voice.premium.en-GB.Serena", name: "Serena", language: "en-GB", quality: "Premium" },
        { id: "com.apple.voice.compact.en-US.Samantha", name: "Samantha", language: "en-US", quality: "" },
        { id: "com.apple.voice.enhanced.de-DE.Anna", name: "Anna", language: "de-DE", quality: "Enhanced" },
      ];
    case "reading_try":
      return null;
    case "forget_learned_word":
      return null;
    case "style_profiles":
      return [
        { kind: "email", label: "Email", profile: "Open with “Hi” and the first name.\nSign off with “Cheers, Alex”.\nKeep it to a few short sentences.", edits: 7 },
        { kind: "chat", label: "Work chat", profile: "", edits: 2 },
        { kind: "texts", label: "Texts", profile: "", edits: 0 },
        { kind: "other", label: "Everything else", profile: "", edits: 0 },
      ];
    case "style_set_profile":
    case "style_forget":
      return null;
    case "get_recent_apps":
      return [{ id: "com.google.Chrome", name: "Google Chrome", site: "docs.google.com" }, { id: "com.linear", name: "Linear", site: null }];
    case "get_insights":
      return {
        today_words: 412, week_words: 3870, total_words: 52140, total_dictations: 1306, speaking_wpm: 146, minutes_saved: 951, streak_days: 12, typing_wpm: 40,
        chart: Array.from({ length: 30 }, (_, i) => ({ date: new Date(Date.now() - (29 - i) * 864e5).toISOString().slice(0, 10), words: [0, 6].includes(new Date(Date.now() - (29 - i) * 864e5).getDay()) ? 80 + i * 3 : 600 + ((i * 137) % 900) })),
        top_apps: [["Slack", 9120], ["Claude", 7400], ["Mail", 3150], ["Messages", 1980], ["Cursor", 1240]],
      };
    case "app_info":
      return args.ids.map((id) => ({ id, name: { "com.apple.mail": "Mail", "com.tinyspeck.slackmacgap": "Slack", "com.apple.MobileSMS": "Messages", "com.apple.Terminal": "Terminal", "com.google.Chrome": "Google Chrome", "com.linear": "Linear" }[id] ?? null, icon: null }));
    case "try_preset":
      return "Hey, can we move the call to Thursday?";
    case "redo_history":
      return "Hi Sam — quick heads-up: the deploy moved to Thursday.";
    case "get_history":
      return [
        { at: Date.now() - 60000, app: "Mail", mode: "dictate_instruct", raw: "hey sam um the deploy slipped to thursday", instruction: "make it a short email", text: "Hi Sam,\n\nQuick update: the deploy has slipped to Thursday.\n\nThanks,\nAlex", note: null, stt: "local/nemotron-speech-streaming-en-0.6b-Q4_K_M", llm: "openrouter/google/gemini-2.5-flash-lite", millis: 1400 },
        { at: Date.now() - 3600000, app: "Slack", mode: "dictate", raw: "sounds good I'll take a look after lunch", instruction: null, text: "Sounds good, I'll take a look after lunch.", note: null, stt: null, llm: null, millis: 900 },
      ].filter((e) => !args.query || e.text.toLowerCase().includes(args.query.toLowerCase()));
    case "notes_folder":
      return { path: "/Users/alex/Documents/Splashdown Notes", label: "Documents › Splashdown Notes", synced: null, problem: null };
    case "notes_cloud_places":
      return {
        cloud: [
          { service: "iCloud Drive", account: null, folder: "/Users/alex/Library/Mobile Documents/com~apple~CloudDocs/Splashdown Notes", label: "iCloud Drive › Splashdown Notes" },
          { service: "Dropbox", account: null, folder: "/Users/alex/Library/CloudStorage/Dropbox/Splashdown Notes", label: "Dropbox › Splashdown Notes" },
        ],
        local: { service: "this computer", account: null, folder: "/Users/alex/Documents/Splashdown Notes", label: "Documents › Splashdown Notes" },
      };
    case "notes_phone":
      return { icloud: true, used: false };
    case "voices_list":
      return [{ name: "Priya", meetings: 4 }, { name: "Sam", meetings: 1 }];
    case "voices_forget":
      return null;
    case "notes_watch_waiting":
    case "notes_watch_take_waiting":
      return 3;
    case "notes_watch_pick":
      return "/Users/alex/Movies/Zoom";
    case "export_history":
    case "export_settings":
      return "~/Downloads/Splashdown export.txt";
    case "check_settings_file":
      return { presets: null, snippets: 2, vocabulary: 4, outside: [] };
    case "check_for_updates":
      return mockInvoke.state.update.available;
    case "install_update":
      throw "Updates can't be installed in the preview.";
    case "send_feedback":
      return null;
    case "download_model": {
      // Two seconds of progress, then it's here.
      const total = 2_500_000_000;
      for (let i = 1; i <= 10; i++) {
        await new Promise((r) => setTimeout(r, 200));
        mockEmit("model-download", { id: args.id, done: (total / 10) * i, total });
      }
      for (const list of [mockInvoke.state.speech_models, mockInvoke.state.local_models]) {
        const m = list.find((m) => m.id === args.id);
        if (m) m.downloaded = true;
      }
      mockEmit("model-downloaded", { id: args.id, error: null });
      return { state: mockInvoke.state, engine_error: null };
    }
    case "test_ai":
      return { output: "I think we should ship it on Thursday.", millis: 512, model: "openrouter/google/gemini-2.5-flash-lite", note: null };
    case "import_settings":
      return { state: mockInvoke.state, engine_error: null };
    case "get_launch_at_login":
    case "calendar_access":
    case "reminders_access":
      return false;
    case "reminders_has_list":
      return args.name.toLowerCase() !== "no such list";
    case "notes_meaning_progress":
      return [0, 0];
    case "coding_agents":
      return { server: "/Applications/Splashdown.app/Contents/MacOS/splashdown-mcp", agents: [] };
    case "save_key":
    case "delete_key": {
      const key = mockInvoke.state.keys.find((k) => k.name === args.name);
      if (key) Object.assign(key, cmd === "save_key" ? { source: "keychain", hint: args.value.slice(-4) } : { source: null, hint: null });
      return { state: mockInvoke.state, engine_error: null };
    }
    case "prepare_apple_speech":
      return { state: mockInvoke.state, engine_error: null };
    case "retranscribe_history":
      return { text: "Hi Sam, quick update: the deploy has slipped to Thursday.", stt: "local/nemotron-speech-streaming-en-0.6b-Q4_K_M" };
    case "import_words":
      return { settings: args.settings, kind: "dictionary export", vocabulary: 12, replacements: 3, snippets: 1, duplicates: 2, conflicts: 1, skipped: 0 };
    case "read_preset_file":
      return { id: "shared", name: "Release notes", instructions: "Short bullet points, newest first.", ai: true, insert: "paste", rules: [] };
    case "export_preset":
      return `~/Downloads/${args.preset.name}.toml`;
    case "notes_add":
      return "Inbox";
    case "my_action_problem":
    case "hooks_problem":
    case "hooks_docs_page":
      return null;
    case "hooks_test":
      if (args.hook.kind === "ask" && args.hook.when === "schedule") return "From your notes (5 from the last 7 days), it wrote:\n\n## Done or decided\n\n- The pricing page goes live on Friday (Pricing review).\n\n## Still open\n\n- [ ] Send Dana the final numbers by Wednesday (Pricing review)";
      if (args.hook.kind === "ask" && args.hook.when === "before-meeting") return "From your next meeting with earlier notes, Pricing review, and the last 2 notes about it, it wrote:\n\n**Decided**\n- The pricing page goes live on Friday (Pricing review, 10 November)\n\n**For me to do**\n- [ ] Send Dana the final numbers by Wednesday (Pricing review, 10 November)\n\n**Waiting on others**\n- Sam Reyes will check the EU prices with finance (Pricing review, 10 November)";
      if (args.hook.kind === "ask") return "From a sample, it wrote:\n\nHi Dana,\n\nThanks for today. We agreed the pricing page goes live on Friday.\n\n- I'll send you the final numbers by Wednesday.\n- I'll write to support tomorrow.";
      return args.hook.when === "before-insert" ? "It gave back: “This is a test from Splashdown.”" : "Sent, and the server took it.";
    case "hooks_run_now":
      return `Running “${args.hook.name ?? "the hook"}” on your notes (5 from the last 7 days)`;
    case "hooks_ai":
      return ["the AI on this computer", true];
    case "hooks_runs":
      return [
        { at: "2026-11-09T15:04:00-05:00", event: "meeting", hook: "Follow-up email", about: "Pricing review", ok: true, said: "Added to the note", took: 41.2 },
        { at: "2026-11-09T15:03:00-05:00", event: "meeting", hook: "hooks.zapier.com", about: "Pricing review", ok: false, said: "the server said 410 Gone", took: 0.4 },
      ];
    case "hooks_runs_clear":
      return null;
    case "hooks_secret":
    case "hooks_new_secret":
      return "sds_0123456789abcdef";
    default:
      console.warn(`mock: no sample answer for ${cmd}`);
      return null;
  }
}
