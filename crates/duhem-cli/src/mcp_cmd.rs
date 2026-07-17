//! `duhem mcp` — expose the action contract + `validate` over the Model
//! Context Protocol (stdio), so a bare-chat agent (no repo, no CLI in
//! hand) can author a Verification Definition by *retrieval + verification*
//! rather than from a (nonexistent, wrong-version) pretraining corpus.
//! Spec #251.
//!
//! Three read-only tools — `duhem_actions`, `duhem_describe`,
//! `duhem_validate` — served from the same binary, so they are always
//! version-exact with the installed `duhem`. **No `run` tool**: running a
//! VD needs the real system (SUT + `up:`/`down:` + browser), which stays
//! local / CI, never in an MCP server.
//!
//! Transport: MCP stdio = newline-delimited JSON-RPC 2.0. The surface is
//! tiny, so this is hand-rolled over `serde_json` rather than pulling an
//! SDK; if the tool set grows, swap in a maintained crate.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use serde_json::{Value, json};

use duhem_actions::{catalog, contract_for};
use duhem_schema::{VerificationDefinition, validate};

/// The MCP protocol revision we speak.
const PROTOCOL_VERSION: &str = "2024-11-05";

pub(crate) fn run() -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue; // ignore malformed input rather than crash the session
        };
        // A JSON-RPC notification has no `id` and gets no response
        // (e.g. `notifications/initialized`).
        let Some(id) = req.get("id").cloned() else {
            continue;
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let msg = match dispatch(method, req.get("params")) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        if writeln!(out, "{msg}").is_err() {
            break;
        }
        let _ = out.flush();
    }
    ExitCode::SUCCESS
}

fn dispatch(method: &str, params: Option<&Value>) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "duhem", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_defs() })),
        "tools/call" => call_tool(params),
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

fn tool_defs() -> Value {
    json!([
        {
            "name": "duhem_actions",
            "description": "List the Duhem action catalog: every `uses` string with a one-line summary.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "duhem_describe",
            "description": "Describe one Duhem action's contract — its `with:` fields (with closed enums), the `outputs` it produces, and a worked example. Retrieve this before authoring a check; do not guess field or output names.",
            "inputSchema": {
                "type": "object",
                "properties": { "uses": { "type": "string", "description": "The action `uses` string, e.g. `ui/assert-element` or `api/call`." } },
                "required": ["uses"],
                "additionalProperties": false
            }
        },
        {
            "name": "duhem_validate",
            "description": "Validate a Verification Definition given as YAML text: structural checks plus field-accuracy against the action contract (unknown `with:` keys, wrong `outputs:` fields, invalid enum values). Returns OK, or a list of errors that name the valid options — the verification half of author -> validate -> fix.",
            "inputSchema": {
                "type": "object",
                "properties": { "vd": { "type": "string", "description": "The Verification Definition YAML." } },
                "required": ["vd"],
                "additionalProperties": false
            }
        }
    ])
}

fn call_tool(params: Option<&Value>) -> Result<Value, (i64, String)> {
    let params = params.ok_or((-32602, "missing params".to_string()))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing tool name".to_string()))?;
    let args = params.get("arguments");

    let text = match name {
        "duhem_actions" => actions_text(),
        "duhem_describe" => {
            let uses = str_arg(args, "uses")?;
            describe_text(&uses)
        }
        "duhem_validate" => {
            let vd = str_arg(args, "vd")?;
            validate_text(&vd)
        }
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };

    Ok(json!({ "content": [ { "type": "text", "text": text } ] }))
}

fn str_arg(args: Option<&Value>, key: &str) -> Result<String, (i64, String)> {
    args.and_then(|a| a.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or((-32602, format!("missing string argument `{key}`")))
}

fn actions_text() -> String {
    let mut s = String::new();
    for c in catalog() {
        s.push_str(&format!("{:<18}  {}\n", c.uses, c.summary));
    }
    s
}

fn describe_text(uses: &str) -> String {
    let Some(c) = contract_for(uses) else {
        let known: Vec<&str> = catalog().iter().map(|c| c.uses).collect();
        return format!("unknown action `{uses}`. Known: {}", known.join(", "));
    };
    let mut s = format!("{}\n  {}\n\nwith:\n", c.uses, c.summary);
    for f in &c.with {
        let req = if f.required { "required" } else { "optional" };
        if f.enum_values.is_empty() {
            s.push_str(&format!("  {} ({req})\n", f.name));
        } else {
            s.push_str(&format!(
                "  {} ({req}) — one of: {}\n",
                f.name,
                f.enum_values.join(", ")
            ));
        }
    }
    if c.outputs.is_empty() {
        s.push_str("\noutputs: (none)\n");
    } else {
        s.push_str(&format!("\noutputs: {}\n", c.outputs.join(", ")));
    }
    s.push_str(&format!("\nexample:\n{}\n", c.example));
    s
}

fn validate_text(vd: &str) -> String {
    let def = match VerificationDefinition::from_yaml_str(vd) {
        Ok(d) => d,
        Err(e) => return format!("PARSE ERROR: {e}"),
    };
    let mut errs: Vec<String> = Vec::new();
    if let Err(ve) = validate(&def) {
        errs.extend(ve.iter().map(ToString::to_string));
    }
    errs.extend(crate::contract_check::field_errors(&def));
    if errs.is_empty() {
        "OK — valid".to_string()
    } else {
        format!(
            "INVALID ({} error(s)):\n- {}",
            errs.len(),
            errs.join("\n- ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_reports_server_info_and_tools_capability() {
        let r = dispatch("initialize", None).unwrap();
        assert_eq!(r["serverInfo"]["name"], "duhem");
        assert!(r["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_has_the_three_tools_and_no_run() {
        let r = dispatch("tools/list", None).unwrap();
        let names: Vec<&str> = r["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["duhem_actions", "duhem_describe", "duhem_validate"]);
        assert!(!names.iter().any(|n| n.contains("run")));
    }

    #[test]
    fn describe_tool_returns_the_documented_output() {
        let p = json!({ "name": "duhem_describe", "arguments": { "uses": "ui/assert-element" } });
        let r = call_tool(Some(&p)).unwrap();
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("satisfied"), "{text}");
    }

    #[test]
    fn validate_tool_surfaces_a_field_check_error() {
        let vd = "verification: t\ncriteria:\n  - id: AC-1\n    description: d\n    checks:\n      - id: AC-1.1\n        description: d\n        steps:\n          - { uses: api/call, with: { method: GET, url: u, bogus: 1 } }\n        assertions: [\"1 == 1\"]\n";
        let p = json!({ "name": "duhem_validate", "arguments": { "vd": vd } });
        let r = call_tool(Some(&p)).unwrap();
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("no `with:` field `bogus`"), "{text}");
    }

    #[test]
    fn unknown_method_is_a_jsonrpc_method_not_found() {
        let e = dispatch("frobnicate", None).unwrap_err();
        assert_eq!(e.0, -32601);
    }
}
