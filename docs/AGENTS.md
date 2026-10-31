# Voice input and notes for coding agents

`splashdown-mcp` is a small [MCP](https://modelcontextprotocol.io) server that lets a coding agent ask you a question and hear your answer, and, if you allow it, look things up in your notes.

- **`ask_user_by_voice`:** Splashdown shows the question in its overlay and records hands-free. When you've answered, tap the dictation key and the text goes back to the agent, cleaned up like any dictation. Esc cancels.
- **`search_notes`, `read_note` and `append_note`:** search your notes, read one, or add a paragraph to one (your Inbox or today's note unless the agent names another). **Off until you allow them** in Settings → Privacy → Coding agents: "Let coding agents read notes", and separately "…and add to them". Whatever an agent reads goes to that agent's own AI provider. Until then, the tools answer with how to turn them on.

Splashdown must be running. The server talks to it over the app's control socket, so nothing leaves your computer beyond what your speech and AI providers already see.

## Build it

```sh
cargo build --release -p splashdown-mcp
# → target/release/splashdown-mcp
```

Use the absolute path to that file below. (Or copy it somewhere permanent, such as `~/.local/bin`.)

## Claude Code

```sh
claude mcp add splashdown -- /path/to/splashdown-mcp
```

Add `--scope user` to use it in every project. Then ask Claude to check with you by voice, or add a line to your `CLAUDE.md`, for example: "When you need a decision from me, ask with `ask_user_by_voice`."

## Cursor

In `~/.cursor/mcp.json` (every project) or `.cursor/mcp.json` (one project):

```json
{
  "mcpServers": {
    "splashdown": {
      "command": "/path/to/splashdown-mcp"
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
- Ask in plain words: "search my notes for the launch plan", "note that down in my inbox". Agents only use the notes tools when asked to; nothing is read in the background.
- Notes are only reached inside your notes folder: an agent can't read other files through these tools.
