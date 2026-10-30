# Tracker vocabulary scan

`cargo run -p xtask -- tracker-scrub` reads this repository's issue/PR
bodies and titles, conversation comments, review bodies, and inline review
comments. The daily `tracker vocabulary scan` workflow runs it outside PR
CI; it is also available through workflow_dispatch.

`.github/tracker-scrub-authors.json` explicitly configures author logins.
The maintainer login is included because agents post under that account;
bot accounts are listed explicitly. This also scans that account's human
posts: GitHub cannot distinguish the authoring process. Non-configured
authors and historical third-party mentions are outside the scan.

The scanner reuses the exact matcher and DENY/allow vocabulary used by
`skill-scrub`, without maintaining a second list. It reports artifact URLs,
field names and line numbers. It omits matching words and text from logs
to avoid repeating the content it detects. A hit fails the scheduled job
for review; the scanner never edits, deletes or posts tracker content.

This detects after publication and cannot prevent initial exposure. It
covers only the configured authors on this tracker, not other authors or
other trackers. The report states these limits on every run. The offline
`--fixture PATH` mode reads an array of GitHub-shaped artifact records;
`--authors-file PATH` selects explicit login configuration. Unit tests use
a positive fixture, an allow-listed public slug, and excluded-author cases.
