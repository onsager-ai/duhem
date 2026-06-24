//! `api/*` actions — the HTTP-backed half of the v1 catalog.
//!
//! Two actions ship today: [`Call`] (active HTTP, `api/call`, #21)
//! and [`Observe`] (passive HTTP observation via Playwright network
//! interception, `api/observe`, #38).
//!
//! Both surface the response side under a shared set of output names
//! — `status`, `body`, `body_text`, `headers` — so an author can
//! write assertions like `$steps.x.outputs.status == 201` regardless
//! of whether `x` is an `api/call` or `api/observe` step.
//! `api/observe` adds the request side (`method`, `url`,
//! `request_body`, `request_headers`) that `api/call` doesn't need
//! (its caller specified those in `with:`).
//!
//! Both exercise the real delivery web: real DNS, real TLS, real
//! handler. The Holistic Verification Principle (`CLAUDE.md` /
//! `docs/duhem-spec.md` §8) forbids in-process request mocking.

pub mod call;
pub mod observe;
pub mod poll;

pub use call::Call;
pub use observe::Observe;
pub use poll::Poll;
