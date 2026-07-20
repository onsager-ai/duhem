# Launch posts — teardown demo (DRAFT, pending your sign-off)

Drafts for distributing [`teardown-tautological-tests.md`](./teardown-tautological-tests.md).
Nothing here is published — posting from your accounts is the gate (see
[`README.md`](./README.md)). Where to host the write-up itself: recommend your
blog (marvinzhang.dev), as a companion to the existing *"Your AI Says All Tests
Pass"* post; the links below assume that URL — swap in the real one before posting.

---

## Hacker News

HN rewards a real technical argument with a runnable artifact and punishes
marketing. Lead with the claim, not the product. Submit the **blog post** (not the
repo) so the top-of-page is the argument.

**Title (pick one):**
- `Your AI wrote 200 passing tests. They prove nothing.`
- `The gate can't have a model in it: verifying AI-written code`

**URL:** the blog post (companion to *"Your AI Says All Tests Pass"*).

**First comment (post immediately after submitting — HN convention for author context):**

> Author here. The pattern that pushed me to build this: an agent writes the
> feature and the tests for it from the same mental model, so when the model is
> wrong the tests are wrong in the same direction — green, and meaningless. Even
> the product's own healthcheck can be confidently wrong (a real case in the
> post: `/health` returns `healthy` while nginx serves an empty default page —
> dead product, green check).
>
> The part I'd genuinely like feedback on is the "no LLM in the judge" stance. My
> claim is that an LLM judging LLM-written code shares the blind spot and an agent
> can iterate against it until it's green — so the gate has to be deterministic and
> exercise the whole delivery web, not the unit. There's an 8-file runnable repro
> in the demo/ dir if you want to see it fail then pass. Happy to be argued out of
> the strong version of this.

---

## r/ExperiencedDevs

Promo-sensitive subreddit — a link-drop gets removed. Post as a discussion of the
problem; mention the tool once, at the end, as "what I built after hitting this."
Check the subreddit's self-promotion rule before posting.

**Title:** `AI agents write the code and the tests from the same wrong model — how are you catching the tests that pass but prove nothing?`

**Body:**

> We're all letting agents write tests alongside the code now, and I keep hitting
> the same thing: the suite is green, coverage is high, and none of it means the
> feature works — because the tests were derived from the same mental model as the
> bug. Tautological assertions, mocked-away integrations, coverage without a single
> behavioral check.
>
> The scary version is when the *healthcheck* is the thing that's confidently
> wrong. I had a build where `/health` returned `healthy` while the web front
> served an empty nginx default page — backend fine, product dead, everything
> green. A human clicking through would've shipped it.
>
> How are you gating this on your teams? Interested in what's actually working —
> mutation testing, human-authored acceptance checks, contract tests against the
> real environment, something else? (For what it's worth, what I ended up building
> is a deterministic, no-LLM gate that drives the real environment against
> human-written acceptance criteria — happy to share the runnable demo if useful,
> but I'm more curious what others do.)

---

## QA / testing newsletter pitch (qaskills.sh, DevAssure, etc.)

Short outreach email. One paragraph, one link, one ask.

**Subject:** `Guest teardown: the tests AI agents write that pass but prove nothing`

**Body:**

> Hi [name] — I write about verification for AI-delivered software (Duhem, an
> open-source deterministic merge gate). I've got a teardown your audience would
> recognize instantly: agents write the feature *and* the tests from the same
> mental model, so a wrong model produces green tests that prove nothing — down to
> the healthcheck reporting `healthy` on a dead product. It comes with an 8-file
> runnable repro that fails then passes. Would a contributed piece or a link in
> [newsletter] be a fit? Draft is ready; happy to tailor the framing to your
> readers.

---

## Posting notes (the gate is yours)

- **Publish the write-up first** (your blog), so HN/Reddit link to a real URL, not
  the repo.
- **Sequence:** blog → HN (with the author first-comment) → r/ExperiencedDevs a day
  later (fresh discussion, not an echo) → newsletter pitches in parallel.
- **These are drafts.** Verify the framing reads as *you*, confirm the blog URL,
  and post from your own accounts — that's the human signature this stops at.
