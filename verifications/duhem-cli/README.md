# `duhem-cli`

Duhem-on-Duhem regression coverage (epic #148): drives the **real**
`duhem` binary through the [`cli/invoke`](../../docs/duhem-spec.md)
action and mechanically asserts its command-line contract. Black-box
coverage that complements the white-box cargo tests in
`crates/duhem-cli/tests/`.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Negative fixture: [`fixtures/bad.yml`](fixtures/bad.yml)

## ⚠️ Self-reference caveat — not a trust anchor

This VD is **regression coverage, not independent attestation**. Per
the Asymmetric-trust commitment (`docs/duhem-spec.md` §11.2), the
verifier of AI claims must be structurally independent of the AI making
them. Duhem verifying Duhem is correlated failure: a judge defect that
wrongly passes could equally wrongly pass its own self-test. It is
doubly self-referential — AC-5 runs `duhem run` inside a `duhem run`.
A green run means "no CLI regression detected"; the Onsager seam holds
the trust role.

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | `duhem --version` exits 0 and prints the product version + `schema v…` tag. |
| **AC-2**  | `duhem validate` on a well-formed VD exits 0 with `OK`. |
| **AC-3**  | `duhem validate` on a malformed VD exits non-zero and names the problem on stderr. |
| **AC-4**  | `duhem init` scaffolds a VD that `duhem validate` then accepts. |
| **AC-5**  | `duhem run` on the offline `defaults-example` reaches `verdict: pass`. |

`cli/invoke` runs the **real** binary — no shimmed shell, no fake exit
code (`docs/duhem-spec.md` §8). The exit code is data judged by an
assertion (like `api/call`'s HTTP status), not the action's outcome.

## Operator setup

None beyond a Rust toolchain. Every step uses `cli/invoke`, which is
page-free (`duhem_actions::uses_requires_page`), so **no Playwright
Chromium is needed**. The inner `duhem run` in AC-5 targets the offline
`defaults-example` (a page-free `noop/unobservable` step), so the whole
suite runs green fully offline.

## Running

Run **from the repo root** — `cli/invoke` resolves the relative `cwd`
(default `.`) against the `duhem` process working directory, and the
literal VD paths inside resolve from there.

Against a locally built binary:

```sh
cargo build -p duhem-cli
duhem run verifications/duhem-cli \
  --inputs duhem_bin="$PWD/target/debug/duhem"
```

Against an installed `duhem` (default `duhem_bin=duhem`):

```sh
duhem run verifications/duhem-cli
```

Inputs: `duhem_bin` (binary under test, default `duhem`), `repo_dir`
(invocation cwd, default `.`), `scaffold_dir` (AC-4 scratch dir,
default `target/duhem-cli-regression-scaffold`, gitignored).

## CI

`.github/workflows/self-verify.yml` builds `duhem` and runs this VD on
PRs touching `crates/duhem-cli/**` or `verifications/duhem-cli/**`.
Separate from the Onsager `dogfood` workflow by design (see the caveat
above).

## Status

Proven green end-to-end against a locally built binary: `verdict:
pass`, all five criteria pass. Runs in a couple of seconds, fully
offline, no browser.
