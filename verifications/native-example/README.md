# Native greeting example

Run from the repository root with Node 24 and a built Duhem CLI on PATH:

```sh
duhem validate verifications/native-example
duhem run verifications/native-example
```

The committed greeting project uses Node's built-in `node:test` framework and
JUnit reporter. Node is already used on the Ubuntu self-verification runners;
CI selects Node 24 explicitly. There is no package install, downloaded test
runner, or additional Rust compilation. Pytest is not assumed to be installed.

`greeting-smoke` runs the real CLI and matches its stdout and stderr using
`expect:`. `greeting-unit` runs two real tests and writes `project/junit.xml`.
Duhem reads the JUnit testcase records, so its unit layer reports two passed
tests; the exit-code integration layer counts one command.

The `GREETING` criterion has a separate `cli/invoke` check of the real CLI.
Neither native result satisfies it. Native commands can block the effective
gate while this independently judged body remains passing.

The CLI self-verification suite's `AC-20` drives the negative controls in
[`../duhem-cli/fixtures/native/`](../duhem-cli/fixtures/native/):

- `required.yml` runs a real test with a deliberately wrong expectation. Its
  JUnit report records one failure, the run exits nonzero with verdict `fail`,
  and `body_verdict` stays `pass`.
- `missing-report.yml` runs a successful command that removes the declared
  report. The result is `inconclusive:missing_observation`, with a passing body.
- `diagnostic.yml` runs the same failing test with `required: false`. The native
  failure and count remain visible, while the run exits zero with verdict `pass`.

`.github/workflows/self-verify.yml` validates all four definitions, runs this
positive example, and runs the controls through the CLI suite. Reports are
ignored generated files; each producer overwrites its own report on every run.

For Duhem's own Rust tests, [`../duhem-rust/duhem.yml`](../duhem-rust/duhem.yml)
replaces the direct `cargo test --workspace` step in `.github/workflows/ci.yml`.
CI currently uses cargo test rather than nextest, so this is an exit-code native
run: one native command is counted, rather than individual Rust tests. The
workspace suite executes once and any failed test blocks Duhem's required gate.
This Duhem-on-Duhem coverage is a regression safety net, not an independent
trust anchor.
