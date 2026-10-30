# Owned process example

Requires Linux/macOS, Python 3 and a Duhem build containing #579 (after
v0.7.0). From the repository root, choose a fresh disposable state-file path:

```sh
duhem run verifications/owned-process-example --inputs state_file=/tmp/owned-process-example-state
```

The example starts a real local HTTP server, observes its response and
running state, restarts it with the same state file, observes the incremented
count, explicitly stops it, and inspects the stopped generation. Assertions
live in the VD. The Python fixture only serves facts and writes its startup
count; it does not decide acceptance. Leaf teardown removes the state file
after check-owned processes are finalized.

Port 8769 must be free. Override both `port` and `url` together for another
loopback port. `server_script` defaults to its repository-root relative path;
provide an absolute path if invoking from another directory. Failed checks
still receive required process cleanup. This example does not establish any
consumer application's publication or recovery parity.
