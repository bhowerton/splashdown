# Voice input and notes for coding agents

`splashdown-mcp` is a small [MCP](https://modelcontextprotocol.io) server that lets a coding agent ask you a question and hear your answer, and, if you allow it, look things up in your notes, meetings and transcripts.

- **`ask_user_by_voice`:** Splashdown shows the question in its overlay and records hands-free. When you've answered, tap the dictation key and the text goes back to the agent, cleaned up like any dictation. Esc cancels.
- **`list_notes`:** your notes newest first, narrowed by date (`this_week`, `last_week`, `today`, or `since`/`until` such as `7d` or `2026-11-02`), kind (`meeting`, `transcript`, `capture`, `note`), folder or tag. For meetings it says who spoke, how long it ran and which parts it has, and `with_summary` adds each summary, so "recap my meetings this week" takes one call.
- **`search_notes`:** words in titles and text, with the same filters. A match in a transcript says when and by whom it was said.
- **`read_note`:** a whole note, or one part of a meeting or transcript (`summary`, `my_notes` or `transcript`). A transcript can be read from a time to a time, or for one speaker. Long notes come in pages, so a two-hour transcript can be read to the end.
- **`append_note`:** adds a paragraph to a note (your Inbox or today's note unless the agent names another).

Two **prompts** come with it, which Claude Code offers as commands (`/mcp__splashdown__recap_meetings`, `/mcp__splashdown__action_items`) and other clients list with their prompts:

- **`recap_meetings`** (`when`, default `this_week`): each meeting's decisions and action items, then every action item by person, citing the meetings.
- **`action_items`** (`when`, default `last_7_days`; `person`, optional): the open action items in your meetings and the unticked `- [ ]` to-dos in your other notes, grouped by owner.

Every result has a `splashdown://open?note=` link that opens the note in Splashdown's Notes window (on a Mac), for the agent to point you at.

**The notes tools are off until you allow them** in Settings → Privacy → Coding agents: "Let coding agents read notes", then "…including meetings and transcripts" (what other people said, kept apart because it's theirs) and "…and add to them". To keep one note from agents, choose **Hide from coding agents** in its ⋯ menu (it writes `agents: false` in the note's front matter). Whatever an agent reads goes to that agent's own AI provider. Until then, the tools answer with how to turn them on.

Splashdown must be running. The server talks to it over the app's control socket, so nothing leaves your computer beyond what your speech and AI providers already see.

## Set it up

`splashdown-mcp` comes with the app: on a Mac it's `/Applications/Splashdown.app/Contents/MacOS/splashdown-mcp`, on Windows `splashdown-mcp.exe` next to `Splashdown.exe`. The easiest way in is **Settings → Privacy → Coding agents → Set up an agent**, which lists the agents on your computer with a **Set up** button each:

- **Claude Code and Codex:** Splashdown runs their own `claude mcp add` / `codex mcp add` for you (user-wide, so it works in every project). **Copy command** gives you the line to run yourself instead.
- **Cursor and VS Code:** their install link opens, and they ask you to confirm.
- **Claude Desktop:** **Copy setup** copies the lines to add under Settings → Developer → Edit Config; then restart it.

Start a new session in the agent afterwards. The same is there from a terminal, which is handy over SSH or in scripts:

```sh
splashdown-mcp agents                    # which agents are set up
splashdown-mcp install claude-code       # or codex, cursor, vscode, claude-desktop
splashdown-mcp uninstall claude-code
splashdown-mcp config cursor             # what to add by hand
```

(Run the copy inside the app, by its full path above, so agents are pointed at it.) Splashdown never edits an agent's config files itself: the agent's own command or install link does it, and Claude Desktop's is left to you.

Building from source, it's `cargo build --release -p splashdown-mcp` (→ `target/release/splashdown-mcp`), and `scripts/run-mac-app.sh` puts it in the development app.

## Claude Code

```sh
claude mcp add --scope user splashdown -- /Applications/Splashdown.app/Contents/MacOS/splashdown-mcp
```

Without `--scope user` it's only for the folder you run it in. Then ask Claude to check with you by voice, or add a line to your `CLAUDE.md`, for example: "When you need a decision from me, ask with `ask_user_by_voice`."

## Cursor

In `~/.cursor/mcp.json` (every project) or `.cursor/mcp.json` (one project):

```json
{
  "mcpServers": {
    "splashdown": {
      "command": "/Applications/Splashdown.app/Contents/MacOS/splashdown-mcp"
    }
  }
}
```

## Other agents

Any MCP client that runs stdio servers works the same way: the command is the path to `splashdown-mcp`, with no arguments.

## Good to know

- The tool waits until you answer, for up to 10 minutes (Splashdown stops a hands-free recording after 10 minutes, or 3 minutes of silence). Some clients give up on tools sooner; Claude Code's limit can be raised with the `MCP_TOOL_TIMEOUT` environment variable (milliseconds).
- One question at a time. If you're already dictating, the agent is told to ask in the chat instead.
- The answer uses your pinned preset, or the default one, not the preset for the app in front.
- To try it without an agent, run `splashdown-cli listen --prompt "Tabs or spaces?"`, which prints your answer.
- The notes tools have a command-line twin for scripts, and for agents that run commands rather than MCP: `splashdown-cli notes list` (`--when`, `--since`, `--until`, `--kind`, `--folder`, `--tag`, `--summary`), `notes search <words>` and `notes read <path>` (`--section`, `--start`, `--end`, `--speaker`). `--json` prints what the tools return. They ask the running app, so the same Privacy switches apply.
- Ask in plain words: "recap my meetings this week", "what did Dana say about pricing?", "what's tagged #launch since Monday?", "note that down in my inbox". Agents only use the notes tools when asked to; nothing is read in the background.
- Notes are only reached inside your notes folder: an agent can't read other files through these tools.
