# Tracker private-name scan

`cargo run -p xtask -- tracker-scrub` reads this repository's issue/PR
bodies and titles, conversation comments, review bodies and inline review
comments. The daily `tracker vocabulary scan` workflow runs outside PR CI;
it is also available through workflow_dispatch.

`.github/tracker-scrub.json` explicitly configures author logins and an
`updated_on_or_after` UTC date, set to the merge date. The maintainer login
is included because agents post under that account; bot accounts are
listed explicitly. GitHub cannot distinguish human and agent posts under
a shared login. Non-configured authors stay outside the scan.

Untouched artifacts older than the date baseline are ignored. An edit to
an older artifact brings it back into scope. Review bodies use GraphQL's
`updatedAt` because REST's submission date would miss subsequent edits.
The existing skill-template process-vocabulary policy stays unchanged.
The tracker uses a separate, verified private-name policy in the same
shared module. Ordinary process words and public repository names are
not denied by the tracker policy.

Private-name rules use opaque IDs and SHA-256/length signatures rather
than repeating the protected identifiers in source or logs. Adding a
rule requires a current visibility check; remove its signature when its
repository becomes public. This is a way to avoid republishing names,
not a secrecy guarantee for predictable identifiers.

The normal scan reports only artifact URLs, field names and line numbers.
It never logs matching text, edits or deletes artifacts, or posts comments.
It states its limits on every run: detection after publication, the date
baseline, configured authors only, and this tracker only.

`--measure` examines both the unchanged skill policy and the private-name
policy over all configured-author history, independently of the cutoff.
It emits only aggregate matching-line counts per opaque rule ID and UTC
update year, with no artifact URLs or content. `--fixture PATH` reads an
offline array of GitHub-shaped records; `--config PATH` selects explicit
configuration. Tests cover old untouched, old edited, new and excluded
author records, public names, process words and missing timestamps.
