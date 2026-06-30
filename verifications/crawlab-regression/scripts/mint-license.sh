#!/bin/sh
# Mint a fresh HS256 dev/test license JWT for crawlab-pro:develop and print
# it to stdout.
#
# WHY this works (verified recipe, baked in): the crawlab-pro `develop`
# image verifies its license as an HS256 JWT signed with the hardcoded
# DEV key `test-secret`. The claims it checks are `created_at`
# (unix-epoch integer) and `username` (any string); the header is the
# bare `{"alg":"HS256"}` with NO `typ`. So a token we sign ourselves with
# `test-secret` validates against the develop image without any paid
# license server.
#
# This is the maintainer-sanctioned DEV/TEST license for the regression
# cluster (the maintainer owns crawlab-pro). It is NOT a production
# license and must never be presented as one — it only satisfies the
# develop image's dev-key check. The token is minted fresh at provision
# time (current `created_at`), never committed.
#
# Override the embedded username with CRAWLAB_LICENSE_USERNAME.

set -eu

USERNAME="${CRAWLAB_LICENSE_USERNAME:-duhem-regression@onsager.ai}"
KEY="${CRAWLAB_LICENSE_KEY:-test-secret}"

if command -v python3 >/dev/null 2>&1; then
  python3 - "$USERNAME" "$KEY" <<'PY'
import hmac, hashlib, base64, json, time, sys
username, key = sys.argv[1], sys.argv[2]
b = lambda x: base64.urlsafe_b64encode(x).rstrip(b'=').decode()
header = b(json.dumps({"alg": "HS256"}, separators=(',', ':')).encode())
payload = b(json.dumps({"created_at": int(time.time()), "username": username},
                       separators=(',', ':')).encode())
seg = header + '.' + payload
sig = b(hmac.new(key.encode(), seg.encode(), hashlib.sha256).digest())
print(seg + '.' + sig)
PY
elif command -v openssl >/dev/null 2>&1; then
  # POSIX/openssl fallback. base64url = standard base64, +/ -> -_, drop '='.
  b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
  header=$(printf '%s' '{"alg":"HS256"}' | b64url)
  now=$(date +%s)
  payload=$(printf '%s' "{\"created_at\":${now},\"username\":\"${USERNAME}\"}" | b64url)
  seg="${header}.${payload}"
  sig=$(printf '%s' "$seg" | openssl dgst -sha256 -hmac "$KEY" -binary | b64url)
  printf '%s.%s\n' "$seg" "$sig"
else
  echo "mint-license.sh: need python3 or openssl to mint the dev license" >&2
  exit 2
fi
