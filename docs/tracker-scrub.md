# Tracker private-name scan

`cargo run -p xtask -- tracker-scrub` reads this repository's issue/PR
bodies and titles, conversation comments, review bodies and inline review
comments. The daily `tracker vocabulary scan` workflow runs outside PR CI;
it is also available through workflow_dispatch.

`.github/tracker-scrub.json` explicitly configures author logins and an
`updated_on_or_after` UTC timestamp, set during final preparation on the
merge date. Timestamp precision excludes earlier activity on that day. The maintainer login
is included because agents post under that account; bot accounts are
listed explicitly. GitHub cannot distinguish human and agent posts under
a shared login. Non-configured authors stay outside the scan.

Untouched artifacts older than the timestamp baseline are ignored. An edit to
an older artifact brings it back into scope. Review bodies use GraphQL's
`updatedAt` because REST's submission date would miss subsequent edits.
The existing skill-template process-vocabulary policy stays unchanged.
The tracker uses a separate, verified private-name policy in the same
shared module. Ordinary process words and public repository names are
not denied by the tracker policy.

The tracker list in `xtask/src/private_vocab.rs` uses plain terms. The
initial terms, `chreode` and `ostrom-hub`, were checked with
`gh repo view onsager-ai/<name> --json visibility` on 2026-10-30 and both
were PRIVATE. Both names already appear in this repository's public source.
A term enters the list only if it names a repository or product whose
visibility is PRIVATE (or which does not publicly exist), checked with
`gh repo view` at review time. Record the check date with the policy;
remove a term when its repository or product becomes public. Ordinary
process terms and now-public repositories remain outside this policy.

The normal scan reports only artifact URLs, field names and line numbers.
It never logs matching text, edits or deletes artifacts, or posts comments.
Public URLs form an index of hit locations, a trade-off accepted so the
owner can find and fix the already-public artifacts.
It states its limits on every run: detection after publication, the timestamp
baseline, configured authors only, and this tracker only.

`--measure` examines both the unchanged skill policy and the private-name
policy over all configured-author history, independently of the cutoff.
It emits only aggregate matching-line counts per term and UTC
update year, with no artifact URLs or content. `--fixture PATH` reads an
offline array of GitHub-shaped records; `--config PATH` selects explicit
configuration. Tests cover old untouched, old edited, new and excluded
author records, public names, process words, same-day cutoff precision,
edited old reviews and missing timestamps.
