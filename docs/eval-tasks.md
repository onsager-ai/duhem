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

## Add a rubric score

Use [`llm/score`](action-reference.md#llmscore) for qualities that call for
subjective feedback, such as clarity of an explanation. Keep measurable
requirements in mechanical criteria. A score step sends its rubric and explicit
observations to the configured provider. To run the mechanical gate without
contacting score providers, pass `duhem run --no-scores`.

The terminal report and dashboard show **Scores (non-verdict)**. Read
`scores[criterion_id][check_id][step_id]` in a JSON `RunSummary`, or the same
field under `run` in `bundle.json`. Each entry contains `{"value": 7.5}` or
`{"missing_cause": "disabled by --no-scores"}`. Each key shows the last
completed execution of that step. The trace retains earlier iterations and
retry attempts. Runs without recorded scores or missing-score outcomes omit
the block. Scores are separate from assertions and verdict counts.

The [worked scoring example](../verifications/llm-score-example/README.md)
shows scores from both provider shapes, an out-of-range reply and a run with
`--no-scores`. It also includes a live OpenAI definition for manual runs.

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
fail, so Harbor can show which promise failed. Omit the key of a criterion
that is inconclusive in a run whose overall verdict is still `pass` or `fail`;
the run's `duhem` reward still applies. The adapter copies present scores to
`llm_<step_id>` keys and omits missing scores. For a reused step ID or a key
that matches a criterion reward, the suffix becomes the compact JSON array
`[criterion_id,check_id,step_id]`. If that key is taken, the adapter appends
`#` until it is unique. These rewards leave the `duhem` reward unchanged.
The adapter copies the run bundle into the verifier logs so the verdict and
its evidence sit beside the Harbor transcript. Mechanical grading requires
only the local Duhem CLI. Score steps contact their configured providers.

See the [executable worked example](../verifications/eval-task-example/README.md)
for a small CLI candidate, a deliberately failing candidate and the reference
adapter.
