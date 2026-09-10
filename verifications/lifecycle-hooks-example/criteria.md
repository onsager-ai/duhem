# Criterion and check lifecycle-hook criteria

## AC-1 — Criterion- and check-level setup/teardown compose in order

A criterion-level `setup:`/`teardown:` pair brackets every check in the
criterion; a check-level `setup:`/`teardown:` pair brackets only that
check's own `steps:`. Both are non-judging: they contribute no
assertions of their own, only the preconditions and cleanup a check's
real assertions can then rely on. AC-1.1/AC-1.2 use them for exactly
that — a side effect the check's own steps re-derive independently,
never reading the setup step's output. AC-1.3 contrasts the value case:
when a check needs to *read* a produced value, only leaf-level `setup:`
addresses it from the check body — criterion- and check-level `setup:`
deliberately do not.
