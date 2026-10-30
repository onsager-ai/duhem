//! `cli/*` actions — drive a real command-line program.
//!
//! One action ships today: [`Invoke`] (`cli/invoke`, #102), which runs
//! a command in the SUT environment and captures `exit_code` / `stdout`
//! / `stderr` for assertions.
//!
//! Exercises the real binary — no shimmed shell, no fake exit code —
//! per the Holistic Verification Principle (`docs/duhem-spec.md` §8).

pub mod invoke;

pub use invoke::Invoke;
