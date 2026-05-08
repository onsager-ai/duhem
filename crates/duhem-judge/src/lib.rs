//! Deterministic verdict aggregator for verification check results.
//!
//! Per `docs/duhem-spec.md` §7.6 / §11.2: judgment is mechanical, not
//! LLM-driven. Given a set of structured assertion outcomes from the
//! runtime, produce a `pass` / `fail` / `inconclusive` verdict by pure
//! deterministic evaluation. No model in the loop, ever.
//!
//! Carved off as a separate crate from day one (per §11.2 OSS-judge
//! boundary) so the future open-source reference judge can ship without
//! pulling in runtime / actions / evidence machinery. The aggregation
//! rules themselves land in a follow-up spec (`spec(judge):
//! three-state verdict aggregation rules`).
