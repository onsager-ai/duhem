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
The existing skill-template process-vocabulary policy retains its public terms
and additionally applies the runtime private list.
The tracker uses a separate, verified private-name policy in the same
shared module. Ordinary process words and public repository names are
not denied by the tracker policy.

The private list is supplied at runtime through `DUHEM_PRIVATE_NAMES`, a
comma-separated environment variable. CI reads the repository secret of the
same name for the source, published-skill, DX and scheduled tracker gates.
Unset, empty or empty-element lists fail explicitly; tests inject fake names.
For local use, configure the environment privately; do not put the values in
shell history, command arguments, fixtures or committed files. The loader trims
whitespace, normalizes case and deduplicates entries. The public project slugs
`onsager-ai/duhem` and `onsager-ai/duhem-docs` remain allow-listed.

The owner adds a term only if it names a repository or product whose visibility
is PRIVATE (or which does not publicly exist), checked with `gh repo view` at
review time; remove it when it becomes public. The initial visibility review
was on 2026-10-30. Keep names and future review records outside public source.
Ordinary process terms and now-public repositories stay outside this policy.

`cargo run -p xtask -- source-scrub` scans every tracked working-tree file
(including dotfiles, without following symlinks), not git history. It fails on
any private-name hit and prints only paths, line numbers and total counts.
The initial source scrub is forward-only; existing commits are unchanged.

The normal scan reports only artifact URLs, field names and line numbers.
It never logs matching text, edits or deletes artifacts, or posts comments.
Public URLs form an index of hit locations, a trade-off accepted so the
owner can find and fix the already-public artifacts.
It states its limits on every run: detection after publication, the timestamp
baseline, configured authors only, and this tracker only.

`--measure` examines both the unchanged skill policy and the private-name
policy over all configured-author history, independently of the cutoff.
It emits only aggregate matching-line counts per numeric rule position and UTC
update year, with no terms, artifact URLs or content. Positions are zero-based
and scoped to their policy; private positions follow the normalized secret order. `--fixture PATH` reads an
offline array of GitHub-shaped records; `--config PATH` selects explicit
configuration. Tests cover old untouched, old edited, new and excluded
author records, public names, process words, same-day cutoff precision,
edited old reviews and missing timestamps.
