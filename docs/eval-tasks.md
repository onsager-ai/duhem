# Eval tasks as Verification Definitions

An eval task asks whether a candidate change fulfills its brief. Model and agent
runs can use Duhem as the deterministic per-run verifier, while Harbor owns the
sandbox, agent execution, repeats, transcripts, tokens and timing. Each run
checks the same frozen Verification Definition (VD) against the candidate
workspace.

## Turn the brief into criteria

Treat the task brief as the source of intent. Make one criterion for each
promise the brief makes, with a `description` that restates that promise. Keep
the criteria stable as candidate implementations change; derive checks from
the criteria and revise checks when needed to test behavior through existing
actions such as `cli/invoke`, `api/*`, `db/*`, `process/*` and `ui/*`.

Author and review the VD before any agent run. Keep it in the task's `tests/`
directory, outside the agent sandbox, so candidates cannot see or change their
grader. Harbor builds the base commit and supplies the candidate workspace as
`$inputs.workspace`; keep `env: TASK_WORKSPACE` as a fallback for local runs.
Builds may gate the run through `native:` or check setup, but a build alone does
not satisfy a criterion.

When a library-only change has no command, HTTP or data surface, the brief must
name the public API it promises. Add a probe crate written from that brief and
exercise it with `cli/invoke`. If the brief names no API and nothing observable,
leave the task ungraded by Duhem. Do not add an API to the brief after seeing
the candidate implementation.

Freeze the VD before running candidates. If the reference solution fails a
frozen VD, treat that as ambiguity in the brief or VD for a human to settle.
Never fit the check to the reference solution. Record human resolutions and
their reasons in the VD's history.

## Harbor adapter contract

Install Duhem at an exact pinned version when building the Harbor image; for
example, `npm install --global duhem@0.9.3`. The task's `tests/test.sh` invokes
`duhem run "$VD" --reporter json`, reads the leaf `RunSummary`, and writes
`reward.json` according to the effective verdict:

| Duhem verdict | Harbor result |
| --- | --- |
| `pass` | `{"duhem": 1, ...}` |
| `fail` | `{"duhem": 0, ...}` |
| `inconclusive` | Write no reward file and exit non-zero; Harbor records a verifier error, never a score of 0. |

Include one `reward.json` key per criterion, with `1` for pass and `0` for
fail, so Harbor can show which promise failed. Keep any LLM grader scores under
separate `llm_*` keys written by a separate grader; they never contribute to
Duhem's verdict or `duhem` reward. Copy the run bundle into the verifier logs
so the verdict and its evidence sit beside the Harbor transcript. Do not
require `duhem server` or extra network access for grading.

See the [executable worked example](../verifications/eval-task-example/README.md)
for a small CLI candidate, a deliberately failing candidate and the reference
adapter.
