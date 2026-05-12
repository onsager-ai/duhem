//! `api/*` actions — the HTTP-backed half of the v1 catalog.
//!
//! Only [`Call`] (`api/call`) ships in v1. `api/observe` (passive
//! sniffing of requests the browser triggers) is documented in
//! `docs/duhem-spec.md` §10.5 and deferred to its own spec because
//! it requires Playwright `Route` / network-interception plumbing
//! that has no analogue here.
//!
//! `api/call` exercises a real HTTP server: real DNS, real TLS, real
//! handler. The Holistic Verification Principle (`CLAUDE.md` /
//! `docs/duhem-spec.md` §8) forbids in-process request mocking.

pub mod call;

pub use call::Call;
