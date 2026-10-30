//! Contracts for runtime-owned processes. Execution needs a declaring scope,
//! so the runtime dispatch boundary implements these rather than `ActionCtx`.
use crate::{ActionContract, FieldSpec};

pub fn contracts() -> Vec<ActionContract> {
    vec![
        ActionContract {
            uses: "process/start",
            summary: "Start an owned local process with optional bounded HTTP or log readiness.",
            with: vec![
                FieldSpec::required("command"),
                FieldSpec::optional("args"),
                FieldSpec::optional("cwd"),
                FieldSpec::optional("env"),
                FieldSpec::optional("ready"),
                FieldSpec::optional("stop_timeout"),
                FieldSpec::optional("kill_timeout"),
            ],
            outputs: vec!["handle", "generation", "pid", "ready"],
            secret_outputs: vec![],
            example: "- id: server\n  uses: process/start\n  with:\n    command: ./server\n    args: [serve]\n    ready:\n      http: { url: http://127.0.0.1:8080/health, status: 200 }\n      timeout: 10s",
        },
        ActionContract {
            uses: "process/inspect",
            summary: "Read an owned process's exit facts and bounded, masked stdout/stderr logs.",
            with: vec![FieldSpec::required("handle"), FieldSpec::optional("cursor")],
            outputs: vec![
                "state",
                "running",
                "exit_code",
                "signal",
                "logs",
                "next_cursor",
                "truncated",
                "dropped_bytes",
                "generation",
                "stream_errors",
            ],
            secret_outputs: vec![],
            example: "- id: status\n  uses: process/inspect\n  with: { handle: $setup.server.outputs.handle }",
        },
        ActionContract {
            uses: "process/stop",
            summary: "Stop and reap a current-check or fixture process group with bounded escalation.",
            with: vec![FieldSpec::required("handle")],
            outputs: vec![
                "stopped",
                "already_exited",
                "exit_code",
                "signal",
                "escalated",
            ],
            secret_outputs: vec![],
            example: "- uses: process/stop\n  with: { handle: $setup.server.outputs.handle }",
        },
        ActionContract {
            uses: "process/restart",
            summary: "Stop the old generation before restarting a current-check or fixture process.",
            with: vec![FieldSpec::required("handle")],
            outputs: vec!["handle", "generation", "pid", "ready"],
            secret_outputs: vec![],
            example: "- id: replacement\n  uses: process/restart\n  with: { handle: $setup.server.outputs.handle }",
        },
    ]
}
