//! Append-only run trace + blob writer for verification evidence.
//!
//! Every action invocation, every assertion outcome, every artifact
//! the runtime touches gets recorded here so a verdict can be
//! independently reconstructed from the trace alone — the
//! reproducibility floor under §11.2's mechanical-judgment commitment.
//!
//! Append-only; never mutated after write. Schema and on-disk format
//! land in `spec(evidence): append-only run trace v1`.
