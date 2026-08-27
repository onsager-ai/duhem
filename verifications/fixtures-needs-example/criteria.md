# Fixture lifecycle criteria

## AC-1 — Isolated resource lifetime

Every check that requests a temporary resource receives one before its work
begins, and the resource is removed after the check completes.
