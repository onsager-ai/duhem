# Tracker vocabulary scan

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
The scan applies the skill-template process-vocabulary policy
(`xtask/src/private_vocab.rs`); the public project slugs
`onsager-ai/duhem` and `onsager-ai/duhem-docs` are allow-listed.

The tracked source tree is scrubbed of private consumer names on a
forward-only basis: existing commits are unchanged, and reintroduction is left
to review. No scanner or secret holds those names.

The normal scan reports only artifact URLs, field names and line numbers.
It never logs matching text, edits or deletes artifacts, or posts comments.
Public URLs form an index of hit locations, a trade-off accepted so the
owner can find and fix the already-public artifacts.
It states its limits on every run: detection after publication, the timestamp
baseline, configured authors only, and this tracker only.

`--measure` examines the vocabulary policy over all configured-author history,
independently of the cutoff. It emits only aggregate matching-line counts per
term and UTC update year, with no artifact URLs or content. `--fixture PATH`
reads an offline array of GitHub-shaped records; `--config PATH` selects
explicit configuration. Tests cover old untouched, old edited, new and excluded
author records, public slugs, same-day cutoff precision, edited old reviews and
missing timestamps.
