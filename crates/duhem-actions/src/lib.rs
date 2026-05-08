//! Built-in action library for verification checks.
//!
//! Houses the canonical action types — `ui/*`, `api/*`, `db/*`, etc. —
//! that Verification Definitions reference. UI actions drive a real
//! browser via `playwright-rs` directly from this crate; there is no
//! Node sidecar (resolved on issue #5; detail in `spec(actions): ui/*`).
//!
//! Action authoring lives here; the executor (which sequences action
//! invocations into a check) lives in `duhem-runtime`.
