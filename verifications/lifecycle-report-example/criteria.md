# AC-1 — Lifecycle execution remains inspectable

A verification report shows which preparation and cleanup blocks actually ran,
including their scope and outcome. Cleanup evidence remains separate from the
artifact verdict, so a failed teardown cannot rewrite a passing check.
