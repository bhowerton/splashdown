//! The MCP messages this server understands, as pure functions of the
//! incoming line, so they can be tested without stdio or the app.
//!
//! MCP over stdio is JSON-RPC 2.0, one message per line. We implement
//! `initialize`, `ping`, `tools/list` and `tools/call` (asking the user by
//! voice, and the user's notes), and honor
//! `notifications/cancelled` (the protocol's spelling).

use serde_json::{Value, json};
use splashdown_control::{NoteFilter, ReadPart, Request};

/// Newest first. A client asking for one of these gets it; any other
/// version gets the newest, as the spec's version negotiation says.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const TOOL: &str = "ask_user_by_voice";
pub const LIST_NOTES: &str = "list_notes";
pub const SEARCH_NOTES: &str = "search_notes";
pub const READ_NOTE: &str = "read_note";
pub const APPEND_NOTE: &str = "append_note";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What to do with one incoming line.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Write this message now.
    Send(Value),
    /// Ask the user `question` (slow), then answer request `id` with
    /// [`tool_result`].
    Ask { id: Value, question: String },
    /// Pass `request` to the app (quick), then answer request `id` with
    /// [`tool_result`].
    Control { id: Value, request: Request },
    /// The client gave up on request `id`.
    Canceled(Value),
    /// A notification or a response to us: nothing to send.
    Nothing,
}

pub fn handle(line: &str) -> Step {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Step::Send(error(Value::Null, PARSE_ERROR, &e.to_string())),
    };
    let Some(obj) = msg.as_object() else {
        // Batches were dropped from MCP in 2025-06-18.
        return Step::Send(error(
            Value::Null,
            INVALID_REQUEST,
            "expected one JSON-RPC object",
        ));
    };
    let id = obj.get("id").cloned();
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // A response to a request of ours (we send none), or junk.
        return match id {
            Some(id) if !obj.contains_key("result") && !obj.contains_key("error") => {
                Step::Send(error(id, INVALID_REQUEST, "missing method"))
            }
            _ => Step::Nothing,
        };
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);

    let Some(id) = id else {
        return match method {
            "notifications/cancelled" => params
                .get("requestId")
                .cloned()
                .map_or(Step::Nothing, Step::Canceled),
            _ => Step::Nothing,
        };
    };
    match method {
        "initialize" => Step::Send(response(id, initialize(&params))),
        "ping" => Step::Send(response(id, json!({}))),
        "tools/list" => Step::Send(response(id, json!({ "tools": tools() }))),
        "tools/call" => call(id, &params),
        _ => Step::Send(error(
            id,
            METHOD_NOT_FOUND,
            &format!("method not found: {method}"),
        )),
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "splashdown",
            "title": "Splashdown",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Use ask_user_by_voice when you need the user's input and a spoken answer is quicker than typing: a decision, a preference, a clarification. The notes tools reach the user's Splashdown notes (Markdown files) when the user asks you to, and only if they allowed it in Splashdown's settings. Notes are of four kinds: meeting (a recorded call), transcript (an app's audio, the microphone, a file or a link), capture (dictated into the Inbox or a daily note) and note. A meeting note has My notes (what the user wrote), a Summary, and a transcript of turns, \"[m:ss] Speaker: words\"; \"You\" is the user. Find notes with list_notes (by date, kind, folder or tag) or search_notes (by words), then read one part with read_note. Take paths only from those results, and cite notes by title and date. The user must have the Splashdown app running.",
    })
}

/// What list_notes and search_notes take to narrow the notes down.
fn filter_properties() -> Value {
    json!({
        "when": { "type": "string", "description": "A named range: today, yesterday, this_week, last_week, this_month, last_month, last_7_days or last_30_days." },
        "since": { "type": "string", "description": "From this date or time: 2026-11-02, an RFC 3339 time, today, yesterday, or a while ago (7d, 36h)." },
        "until": { "type": "string", "description": "Before this date or time, in the same forms as since." },
        "kind": { "type": "string", "enum": ["meeting", "transcript", "capture", "note"], "description": "Only this kind of note." },
        "folder": { "type": "string", "description": "Only notes in this folder or its subfolders." },
        "tag": { "type": "string", "description": "Only notes with this tag, or one nested under it (work matches work/acme)." }
    })
}

fn with_filters(mut properties: Value) -> Value {
    if let (Some(props), Some(filters)) =
        (properties.as_object_mut(), filter_properties().as_object())
    {
        props.extend(filters.clone());
    }
    properties
}

fn read_only() -> Value {
    json!({ "readOnlyHint": true, "openWorldHint": false })
}

fn tools() -> Value {
    json!([
        ask_tool(),
        {
            "name": LIST_NOTES,
            "title": "List the user's notes",
            "description": "List the user's Splashdown notes, newest first, narrowed by date, kind, folder or tag: \"my meetings this week\", \"notes tagged launch since Monday\". Returns each note's path, title, kind, dates, folder, tags and link; for meetings and transcripts also speakers, duration and which sections exist. with_summary adds each meeting's summary, for a recap in one call. Pages with cursor.",
            "inputSchema": {
                "type": "object",
                "properties": with_filters(json!({
                    "with_summary": { "type": "boolean", "description": "Add each meeting's summary (up to 2,000 characters). At most 25 notes then." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Most notes to return (default 20)." },
                    "cursor": { "type": "string", "description": "next_cursor from the previous page." }
                }))
            },
            "annotations": read_only()
        },
        {
            "name": SEARCH_NOTES,
            "title": "Search the user's notes",
            "description": "Search the user's Splashdown notes for a word or phrase (any case) in titles and text, transcripts included, optionally narrowed like list_notes. Returns up to `limit` notes, title matches first then newest, each with its path, title, kind, dates, a short excerpt, and for transcripts `said`: when and by whom the first matching turns were said, to read from with read_note's start.",
            "inputSchema": {
                "type": "object",
                "properties": with_filters(json!({
                    "query": { "type": "string", "description": "Text to look for. Empty lists the newest notes." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Most notes to return (default 10)." }
                })),
                "required": ["query"]
            },
            "annotations": read_only()
        },
        {
            "name": READ_NOTE,
            "title": "Read a note",
            "description": "Read one of the user's notes by the path list_notes or search_notes gave: all of it, or one section of a meeting or transcript (summary, my_notes or transcript). A transcript can be narrowed to a stretch of time (start, end) or one speaker. Long text comes in pages: pass next_cursor back as cursor for the rest.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The note's path, as list_notes or search_notes returned it." },
                    "section": { "type": "string", "enum": ["all", "summary", "my_notes", "transcript"], "description": "Which part (default all)." },
                    "start": { "type": "string", "description": "Transcript only: from this time in the recording (1:05, 1:02:30 or seconds)." },
                    "end": { "type": "string", "description": "Transcript only: up to this time." },
                    "speaker": { "type": "string", "description": "Transcript only: what this speaker said (You is the user)." },
                    "max_chars": { "type": "integer", "minimum": 500, "maximum": 40000, "description": "Most characters to return (default 20,000)." },
                    "cursor": { "type": "string", "description": "next_cursor from the previous read of the same note and section." }
                },
                "required": ["path"]
            },
            "annotations": read_only()
        },
        {
            "name": APPEND_NOTE,
            "title": "Add to the user's notes",
            "description": "Add a paragraph to the end of one of the user's notes: their capture note (Inbox or today's note) unless `to` names one. It's marked with the time. Use only when the user asks you to note something down. Works only if the user allowed coding agents to add to notes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "What to add, as Markdown." },
                    "to": { "type": "string", "description": "A note's path from list_notes or search_notes. Leave out for the capture note." }
                },
                "required": ["text"]
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        }
    ])
}

fn ask_tool() -> Value {
    json!({
        "name": TOOL,
        "title": "Ask the user by voice",
        "description": "Ask the user a question and get their spoken answer as text. Splashdown shows the question on screen and records until the user taps their dictation key (Esc cancels); the answer is transcribed and lightly cleaned up. Blocks until the user answers (at most 10 minutes). Use for decisions, preferences and clarifications; keep the question short.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to show the user, in a sentence."
                }
            },
            "required": ["question"]
        }
    })
}

fn call(id: Value, params: &Value) -> Step {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("(none)");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    // An input mistake goes back to the model as a tool error, so it can
    // correct itself.
    let missing = |key: &str| {
        Step::Send(response(
            id.clone(),
            tool_result(Err(format!("`{key}` is required"))),
        ))
    };
    let request = match name {
        TOOL => {
            let question = text("question");
            if question.is_empty() {
                return missing("question");
            }
            return Step::Ask { id, question };
        }
        LIST_NOTES => Request::ListNotes {
            filter: filter(&args),
            with_summary: args
                .get("with_summary")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            limit: number(&args, "limit"),
            cursor: optional(&args, "cursor"),
        },
        SEARCH_NOTES => Request::SearchNotes {
            query: text("query"),
            limit: number(&args, "limit"),
            filter: filter(&args),
        },
        READ_NOTE => {
            let path = text("path");
            if path.is_empty() {
                return missing("path");
            }
            Request::ReadNote {
                path,
                part: ReadPart {
                    section: optional(&args, "section"),
                    start: optional(&args, "start"),
                    end: optional(&args, "end"),
                    speaker: optional(&args, "speaker"),
                    max_chars: number(&args, "max_chars"),
                    cursor: optional(&args, "cursor"),
                },
            }
        }
        APPEND_NOTE => {
            let note = text("text");
            if note.is_empty() {
                return missing("text");
            }
            let to = text("to");
            Request::AppendNote {
                text: note,
                to: (!to.is_empty()).then_some(to),
            }
        }
        _ => return Step::Send(error(id, INVALID_PARAMS, &format!("unknown tool: {name}"))),
    };
    Step::Control { id, request }
}

/// A string argument, trimmed; `None` when it's missing or empty. Numbers
/// are taken as their text (a model may send a cursor or time as one).
fn optional(args: &Value, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn number(args: &Value, key: &str) -> Option<usize> {
    let value = args.get(key)?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .and_then(|n| usize::try_from(n).ok())
}

fn filter(args: &Value) -> NoteFilter {
    NoteFilter {
        when: optional(args, "when"),
        since: optional(args, "since"),
        until: optional(args, "until"),
        kind: optional(args, "kind"),
        folder: optional(args, "folder"),
        tag: optional(args, "tag"),
    }
}

/// The `tools/call` result for an answer, or for why there isn't one.
pub fn tool_result(answer: Result<String, String>) -> Value {
    let (text, is_error) = match answer {
        Ok(text) => (text, false),
        Err(e) => (e, true),
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

pub fn response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(line: &str) -> Value {
        match handle(line) {
            Step::Send(v) => v,
            other => panic!("expected a message, got {other:?}"),
        }
    }

    #[test]
    fn initialize_negotiates_the_version() {
        let r = send(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        );
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "splashdown");
        assert!(r["result"]["capabilities"]["tools"].is_object());

        let r = send(
            r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );
        assert_eq!(r["id"], "a");
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn lists_the_tools() {
        let r = send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [TOOL, LIST_NOTES, SEARCH_NOTES, READ_NOTE, APPEND_NOTE]
        );
        assert_eq!(tools[0]["inputSchema"]["required"], json!(["question"]));
        // list and search take the same filters.
        for tool in &tools[1..3] {
            for key in ["when", "since", "until", "kind", "folder", "tag"] {
                assert!(tool["inputSchema"]["properties"][key].is_object(), "{key}");
            }
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
        }
        assert_eq!(tools[4]["annotations"]["readOnlyHint"], false);
    }

    #[test]
    fn notes_tools_become_app_requests() {
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"search_notes","arguments":{"query":"launch","limit":3}}}"#
            ),
            Step::Control {
                id: json!(4),
                request: Request::SearchNotes {
                    query: "launch".into(),
                    limit: Some(3),
                    filter: NoteFilter::default(),
                }
            }
        );
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_notes","arguments":{"when":"this_week","kind":"meeting","with_summary":true,"limit":"5","cursor":20,"tag":" "}}}"#
            ),
            Step::Control {
                id: json!(7),
                request: Request::ListNotes {
                    filter: NoteFilter {
                        when: Some("this_week".into()),
                        kind: Some("meeting".into()),
                        ..NoteFilter::default()
                    },
                    with_summary: true,
                    limit: Some(5),
                    cursor: Some("20".into()),
                }
            }
        );
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_note","arguments":{"path":"Meetings/Sync.md","section":"transcript","start":"1:05","speaker":"Dana"}}}"#
            ),
            Step::Control {
                id: json!(8),
                request: Request::ReadNote {
                    path: "Meetings/Sync.md".into(),
                    part: ReadPart {
                        section: Some("transcript".into()),
                        start: Some("1:05".into()),
                        speaker: Some("Dana".into()),
                        ..ReadPart::default()
                    },
                }
            }
        );
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"append_note","arguments":{"text":"ship it","to":""}}}"#
            ),
            Step::Control {
                id: json!(5),
                request: Request::AppendNote {
                    text: "ship it".into(),
                    to: None
                }
            }
        );
        // A missing path is the model's mistake: a tool error, not a crash.
        let r = send(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"read_note","arguments":{}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn tool_calls_ask_the_user() {
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ask_user_by_voice","arguments":{"question":" Tabs or spaces? "}}}"#
            ),
            Step::Ask {
                id: json!(3),
                question: "Tabs or spaces?".into()
            }
        );

        // No question: a tool error the model can fix.
        let r = send(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ask_user_by_voice","arguments":{}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);

        // Unknown tool: a protocol error.
        let r = send(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"rm_rf","arguments":{}}}"#,
        );
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn results_carry_text_and_errors() {
        assert_eq!(
            tool_result(Ok("Spaces.".into())),
            json!({ "content": [{ "type": "text", "text": "Spaces." }], "isError": false })
        );
        assert_eq!(tool_result(Err("x".into()))["isError"], true);
        assert_eq!(
            response(json!(7), json!({}))["jsonrpc"],
            "2.0",
            "every message says jsonrpc 2.0"
        );
    }

    #[test]
    fn notifications_get_no_reply() {
        assert_eq!(
            handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Step::Nothing
        );
        assert_eq!(
            handle(
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3,"reason":"user"}}"#
            ),
            Step::Canceled(json!(3))
        );
        // A response to a request of ours.
        assert_eq!(
            handle(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#),
            Step::Nothing
        );
    }

    #[test]
    fn bad_input_gets_json_rpc_errors() {
        assert_eq!(send("{not json")["error"]["code"], PARSE_ERROR);
        assert_eq!(send("[]")["error"]["code"], INVALID_REQUEST);
        let r = send(r#"{"jsonrpc":"2.0","id":8,"method":"resources/list"}"#);
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(r["id"], 8);
        assert_eq!(
            send(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)["result"],
            json!({})
        );
    }
}
