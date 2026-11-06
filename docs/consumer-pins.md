# Consumer-owned pin currency

Pin checks run in each consumer's own CI (#380, #605). Duhem keeps no
consumer registry, fetches no consumer workflow, and runs no scheduled
consumer-pin job. Each owner supplies the version it actually uses.

From a checkout of the Duhem revision you want to compare against:

```sh
cargo run --locked -p xtask -- consumer-pins --pinned 0.5.1
```

Use a separate checkout directory when calling this from product CI, and
pass that product's pin from its existing configuration. The command needs
Rust and Duhem's source; it is a repository task, not part of the installed
`duhem` binary. There is no token or cross-repository read requirement.
The owner chooses the schedule and whether the result gates upgrades.

The command compares the supplied pin against that checkout's
`duhem_schema::SCHEMA_VERSION`. Only dated release sections participate;
Unreleased entries do not. It reports `[breaking]` references where
`pin < release <= current`, warns when breaking entries are crossed,
and emits a notice for an older pin without a breaking change. An equal
or newer pin is reported as current or newer. `--pinned=v0.5.1` is also
accepted. Missing or malformed versions fail explicitly.

Currency is advisory: a stale pin exits successfully, and GitHub receives
a warning/notice annotation plus a job summary when `GITHUB_STEP_SUMMARY`
is set. Read/parse failures exit nonzero. This check does not run a
Verification Definition or prove compatibility. Consumers still run their
own suites against the pinned version and review migrations before upgrading.
