//! `api/*` actions — the HTTP-backed half of the v1 catalog.
//!
//! Two actions ship today: [`Call`] (active HTTP, `api/call`, #21)
//! and [`Observe`] (passive HTTP observation via Playwright network
//! interception, `api/observe`, #38). Both share an output shape
//! (`status`, response_body, headers, …) so authors can assert
//! against `$steps.<id>.outputs.<name>` regardless of which side of
//! the request produced the traffic.
//!
//! Both exercise the real delivery web: real DNS, real TLS, real
//! handler. The Holistic Verification Principle (`CLAUDE.md` /
//! `docs/duhem-spec.md` §8) forbids in-process request mocking.

pub mod call;
pub mod observe;

pub use call::Call;
pub use observe::Observe;
