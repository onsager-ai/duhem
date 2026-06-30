# Crawlab — node management & metrics (API-009)

Acceptance criteria for Crawlab Pro's node-management and metrics surface,
ported from crawlab-team/crawlab-test's
`specs/api/API-009-node-management-metrics.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an independent
vendor, so the asymmetric-trust seam is real: Duhem authors these checks
against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against a
real licensed crawlab-pro cluster (master + worker + MongoDB) with no mocks
at the web boundary (`docs/duhem-spec.md` §8): node create/delete are not
exposed by the API (nodes self-register), so the cluster topology is read
straight from the database rather than trusted from the API's own count.

## AC-1

The cluster's node topology is real and reported: the node list includes
both a master and a worker, and the database confirms a master node flagged
as master and online plus a registered worker. The cluster is genuinely
multi-node, verified at the store, not just asserted by the API's own count.

## AC-2

Node details and metrics are served: the master node is fetchable by id
with its identifying fields (hostname, ip, status), the all-nodes metrics
endpoint responds, and the master's current-metrics endpoint returns a
metrics payload. The monitoring surface is live, not a stub.

## AC-3

A node property update is durable: setting the master node's description
over REST lands in the database. Node updates are persisted, not merely
acknowledged.
