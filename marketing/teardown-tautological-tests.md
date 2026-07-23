# Your AI wrote 200 passing tests. They prove nothing.

*A teardown of the self-graded-homework problem in AI-delivered software — and the one gate an agent can't pass by trying harder.*

---

An AI coding agent writes your feature. Then it writes the tests for that feature. Both come from the **same** mental model of what the code should do. When that model is right, everything's green. When that model is wrong, everything is *still* green — because the tests were derived from the same wrong idea as the code.

This is the tautology at the center of AI-delivered software. The failure modes have names now:

- **Tautological assertions** — the test re-implements the code it's testing, so it asserts the code does what the code does.
- **Mocked-away logic** — the one integration that would have failed is stubbed out at verification time.
- **Coverage without behavior** — 200 tests, 95% line coverage, and not one of them exercises whether the product actually works.

You end up with a suite that is green, thorough-looking, and epistemically worthless. The agent graded its own homework and gave itself an A.

## Green stopped meaning "it works"

The reason this is dangerous — and not just untidy — is that *every* signal you'd normally trust sits **downstream of the same model that wrote the bug**. The tests: written by the agent. The coverage number: computed over those tests. Even the product's own healthcheck endpoint: written by the agent, and just as capable of being confidently wrong.

Here's a real one.

A build gated its `:edge` Docker image on a check suite before it could promote to `:stable`. One build's gate went red — the environment wouldn't come up. The cause was nearly invisible: a new image shipped without `envsubst`, so the startup script rendered an **empty** nginx config and the entire web front fell back to the default "Welcome to nginx!" page. Dead product.

The part that matters: **the backend was fine, and the product's own healthcheck reported `healthy`.** A human clicking through, or the container's self-check, would have shipped it. Both endpoints answered `200`. Only the *content* of the front betrayed the break.

That's the class of bug this problem produces: a self-masking regression that everything else — including the product's own healthcheck — is calling healthy.

## You can't fix an LLM's blind spot with more LLM

The instinct is to add another model to the loop — an AI reviewer, an AI judge. But an LLM judging LLM-written code shares the failure it's supposed to catch: it can be confidently wrong in exactly the direction the author was, and an agent can iterate against it until it goes green. If the gate has a model in it, the gate is negotiable.

The gate has to be **deterministic** and it has to exercise the **whole delivery web**, not the unit in isolation. That's the [Duhem–Quine thesis](../docs/duhem-brand.md) the tool is named for: a hypothesis (your code) is only meaningful tested *together with* the web that supports it — prompts, configuration, tool wiring, data state, runtime. Test the center square alone and you've verified nothing; the empty-config break lives in the frame, not the unit.

So Duhem does two things, and refuses to do a third:

1. It captures **human** acceptance criteria — what "done" actually means, written and reviewed by a person.
2. It translates them into checks that drive the **real** delivery web end to end and judges the result by **deterministic evaluation of structured assertions**.
3. There is **no LLM in the judging loop.** AI may help author criteria and checks; the verdict is mechanical. It's the one gate an agent can't satisfy by trying harder, because there's no model in it to persuade.

## Watch it catch the exact bug above

A runnable, 8-file reproduction lives in [`demo/self-masking/`](../demo/self-masking/): a tiny service whose `/health` stays green while its web front is quietly broken — both endpoints answer `200`, exactly like the empty-nginx-config case.

```text
$ duhem run          # front is broken; /health still reports healthy
fail
  AC-2::AC-2.1:
    fail  $steps.front.outputs.body.title == "Acme Dashboard"
        (actual "Welcome to nginx!", expected "Acme Dashboard")

# fix shipped — re-run the same gate
$ duhem run
pass
```

`AC-1` checks health and passes — health *is* fine. `AC-2` drives the real front, reads the actual served page, and fails on the content a human would have missed. The healthcheck's green was true and useless. Duhem's red was the only signal in the room that meant anything.

## Try it

```sh
npx duhem --version          # or: npm i -g duhem
```

- **The repro:** [`demo/self-masking/`](../demo/self-masking/) — `./run.sh` shows the red-then-green above, every line real output.
- **Your own check in 5 minutes:** [`docs/getting-started.md`](../docs/getting-started.md) — `duhem init` scaffolds a Verification Definition; write what "done" means, gate merge on the verdict.
- **In CI:** the `duhem/run` composite GitHub Action gates merge/deploy the same way.
- **The longer argument:** [*Your AI Says "All Tests Pass" — But Do They?*](https://marvinzhang.dev/blog/introducing-duhem)

Open source, Apache-2.0. If your agents write your tests, the tests are not your gate. Get a gate that has no model in it.
