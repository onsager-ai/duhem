# Non-verdict rubric scores

Both definitions check that an answer identifies Paris as the capital of France.
They also show how `llm/score` can record feedback on the wording. Use a Duhem
build that includes the score action and reporting from #708.

## Run with the local provider stub

From the repository root on Linux or macOS, with Python 3 and port 8770 free:

```sh
SCORE_EXAMPLE_STUB_KEY=duhem-score-example-dummy-key \
  duhem run verifications/llm-score-example/duhem.yml --reporter json
```

`process/start` starts the fixture on loopback and waits for `/health`. Duhem
stops the owned process when the check ends. The fixture serves Anthropic
Messages and OpenAI Chat Completions responses. Under `scores.AC-1.AC-1.1`,
`anthropic_clarity` has value `8`, `openai_clarity` has value `3`, and
`out_of_range` has a `missing_cause` because the provider replied with `11`.
The fixed responses exercise the transport and report. The assertion on the
answer determines the verdict. The run passes and exits `0`.

Run the same definition with the score steps skipped. The fixture still starts
for setup. All three score entries record `disabled by --no-scores` and the run
still passes with exit `0`.

```sh
duhem run verifications/llm-score-example/duhem.yml --reporter json --no-scores
```

## Run with a live provider

With `OPENAI_API_KEY` already exported in your shell, run the live definition
manually. It sends the rubric and the sample answer to OpenAI and may incur
provider charges. Use `--inputs model=<model-id>` to select another Chat
Completions model that supports temperature `0`.

```sh
: "${OPENAI_API_KEY:?Export an OpenAI API key first}"
duhem run verifications/llm-score-example/real-provider.yml --reporter json
```

Look under `scores.AC-1.AC-1.1.openai_clarity` for the provider's value or a
missing cause. The live definition uses the same assertion and reads the key
through a registered secret input. CI validates the live definition offline.
