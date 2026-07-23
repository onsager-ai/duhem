//! [`Engine`] configuration surface — the `with_*` builders, split
//! out of `runner.rs` so the run loop stays within the per-file prod
//! budget (`xtask check-file-budget`). Child module of `runner`, so
//! it keeps direct access to `Engine`'s private fields.

use super::*;

impl Engine {
    /// Subscribe a live progress sink (#299): the run's evidence
    /// events are teed to `tx` post-commit, in order. Observational
    /// only — a dropped receiver never affects the run.
    pub fn with_progress(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<duhem_evidence::Event>,
    ) -> Self {
        self.progress = Some(tx);
        self
    }

    /// Apply a manifest's `defaults:` block (spec #66): the per-step
    /// `within:` fallback (`timeout`), the inconclusive policy, and the
    /// retry posture. `defaults.environment` is not consumed here (its
    /// `environments:` lookup is out of scope). Absent sub-keys leave
    /// today's behavior in place.
    pub fn with_defaults(mut self, defaults: &duhem_schema::ManifestDefaults) -> Self {
        self.default_within = defaults.timeout.map(Duration::from);
        self.retry = defaults.retry;
        self.inconclusive_policy = match defaults.inconclusive_policy {
            Some(duhem_schema::InconclusivePolicy::Block) | None => InconclusivePolicy::Block,
            Some(duhem_schema::InconclusivePolicy::Warn) => InconclusivePolicy::Warn,
            Some(duhem_schema::InconclusivePolicy::Pass) => InconclusivePolicy::Pass,
        };
        self
    }

    /// Attach the evidence store this engine writes runs into. The
    /// CLI resolves the store (default project DB or `--db` override)
    /// and threads it here; without one, the engine lazily opens the
    /// working copy's default store on first run.
    pub fn with_store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Attach a pre-launched [`RunBrowser`]. The engine doesn't
    /// launch one on its own — the caller controls when the
    /// (heavyweight) Playwright process is started.
    pub fn with_browser(mut self, browser: RunBrowser) -> Self {
        self.browser = Some(browser);
        self
    }

    /// Set the failure-evidence capture posture (spec #202). Default
    /// is [`CapturePolicy::OnFailure`].
    pub fn with_capture(mut self, capture: CapturePolicy) -> Self {
        self.capture = capture;
        self
    }

    /// Record the source path / identifier of the Verification
    /// Definition for evidence. The CLI threads the file path here;
    /// programmatic callers can pass any stable identifier.
    pub fn with_definition_path(mut self, path: impl Into<String>) -> Self {
        self.definition_path = Some(path.into());
        self
    }

    /// Attach a [`CheckFilter`]. With a filter set, checks for which
    /// `matches(criterion_id, check_id)` returns `false` are skipped
    /// entirely — no events, no verdict slot. Spec on issue #23.
    pub fn with_filter(mut self, filter: impl CheckFilter + 'static) -> Self {
        self.filter = Some(Box::new(filter));
        self
    }

    /// Seed the runtime's entropy source so `$runtime.uuid()` is
    /// derived deterministically from `seed`. Two runs with the same
    /// seed see identical uuid output (spec on issue #33).
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Pin the run id instead of minting a fresh ULID (#189). For
    /// fixtures and tests that need deterministic run URLs; colliding
    /// with an existing run fails at `begin_run`.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Attach scoping + provenance to recorded runs (#190): the
    /// project hint and `verifier_repo@sha VERIFIES target_repo@sha`.
    /// The identity-resolution ladder that computes these is #191's;
    /// this only records what the caller resolved.
    pub fn with_scope(mut self, scope: RunScope) -> Self {
        self.scope = scope;
        self
    }

    /// Skip `environment.up:` + readiness probing. Used by the CLI's
    /// `--no-env-up` flag; useful when the operator brought the SUT
    /// up out-of-band. Teardown still runs unless [`Engine::keep_env`]
    /// is also set — that combination is the "do absolutely no
    /// lifecycle plumbing" debug shape.
    pub fn skip_env_up(mut self, skip: bool) -> Self {
        self.skip_env_up = skip;
        self
    }

    /// Skip `environment.down:`. Used by the CLI's `--keep-env` flag
    /// so an author can poke at the SUT after a failing run.
    pub fn keep_env(mut self, keep: bool) -> Self {
        self.keep_env = keep;
        self
    }

    /// Seed the `$env.<key>` whitelist for this run (spec #68). The CLI
    /// passes the selected named environment's string-valued keys here;
    /// `$env.<key>` resolves against this map. Empty by default, so
    /// runs without a selected environment keep today's behavior (no
    /// `$env` access).
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Declare the leaf's inherited input names (spec #135). When a
    /// referenced `$inputs.<name>` for one of these names resolves to
    /// nothing, the run fails with a loud, specific error naming it as
    /// inherited and pointing at the suite / `--inputs` remedy, instead
    /// of a generic deep failure. The CLI threads `def.inherits` here.
    pub fn with_inherited(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.inherited = names.into_iter().collect();
        self
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
