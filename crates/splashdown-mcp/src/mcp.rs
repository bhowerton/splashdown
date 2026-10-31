//! The MCP messages this server understands, as pure functions of the
//! incoming line, so they can be tested without stdio or the app.
//!
//! MCP over stdio is JSON-RPC 2.0, one message per line. We implement
//! `initialize`, `ping`, `tools/list` and `tools/call` with a single tool,
//! and honor `notifications/cancelled` (the protocol's spelling).

use serde_json::{Value, json};

/// Newest first. A client asking for one of these gets it; any other
/// version gets the newest, as the spec's version negotiation says.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const TOOL: &str = "ask_user_by_voice";

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
        "tools/list" => Step::Send(response(id, json!({ "tools": [tool()] }))),
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
        "instructions": "Use ask_user_by_voice when you need the user's input and a spoken answer is quicker than typing: a decision, a preference, a clarification. The user must have the Splashdown app running.",
    })
}

fn tool() -> Value {
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
    let name = params.get("name").and_then(Value::as_str);
    if name != Some(TOOL) {
        let name = name.unwrap_or("(none)");
        return Step::Send(error(id, INVALID_PARAMS, &format!("unknown tool: {name}")));
    }
    let question = params
        .get("arguments")
        .and_then(|a| a.get("question"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if question.is_empty() {
        // An input mistake goes back to the model as a tool error, so it
        // can correct itself.
        let result = tool_result(Err("`question` is required".into()));
        return Step::Send(response(id, result));
    }
    Step::Ask {
        id,
        question: question.to_string(),
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
    fn lists_the_tool() {
        let r = send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], TOOL);
        assert_eq!(tools[0]["inputSchema"]["required"], json!(["question"]));
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
