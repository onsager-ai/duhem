# Check-level summary totals

This worked Verification Definition for spec #493 deliberately runs
three checks with different verdicts: one pass, one fail, and one
inconclusive. It exercises the default reporter's final aggregate:

```text
Total: 3 checks · 1 passed · 1 failed · 1 inconclusive
```

Validate it offline:

```sh
duhem validate verifications/check-totals-example/duhem.yml
```

Run it to inspect the mixed summary (a non-zero exit is expected because
one check intentionally fails):

```sh
duhem run verifications/check-totals-example/duhem.yml
```

The inconclusive path invokes a genuinely unavailable local process;
there is no mock and every verdict is mechanically judged.
