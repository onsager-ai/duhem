# for_each batch-cleanup criteria

## AC-1 — `for_each:` in `setup:`/`teardown:` replaces hand-written repeated steps

`setup:` creates one marker file per element of the `scratch_ids`
input with a single `for_each:` step (`uses:` body form, `as: id`
bound to the current element); `teardown:` removes every one of them
with a mirror `for_each:` step. Both are non-judging lifecycle
contexts, so the loop can never shrink what a check claims to verify
— that risk only exists once `for_each:` is allowed to wrap judging
steps inside a check (Tier 2, gated behind #509). AC-1.1 and AC-1.2
each independently confirm a marker the loop created — one at each
end of `scratch_ids` — without needing to see how many elements the
array held.
