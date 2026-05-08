//! Verification Definition types and validator.
//!
//! Owns the on-the-wire shape of a Verification Definition — criteria
//! (the human commitment about what "done" means) and checks (the
//! mechanically-judged assertions that verify it). Per
//! `docs/duhem-spec.md` §7.2 / §7.3, criteria are stable; checks are
//! derivative.
//!
//! This crate is a Phase 0 skeleton. The v0.1 type set is the subject
//! of a follow-up spec (`spec(schema): Verification Definition v0.1
//! types`).
