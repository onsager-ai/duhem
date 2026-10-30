# AC-1 — A local service can restart without losing its persistent state

The service responds while owned by the verification, preserves its startup
count across a restart, and is stopped before the check finishes. Recorded
generation, readiness, response and stop facts establish this narrow claim.
