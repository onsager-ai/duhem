# Self-masking demo

A tiny, self-contained reproduction of the pattern in the main README's
proof story: a service whose **health endpoint stays green while its web
front is quietly broken**. Both endpoints answer `200` — only the
*content* of the front betrays the break, exactly like an empty nginx
config falling back to a default page. A real `duhem run` catches it;
the same gate then confirms the fix.

## Run it

Needs `node` and `duhem` on your `PATH` (from a source checkout,
`DUHEM=./target/debug/duhem`).

```sh
./run.sh
```

You'll see the gate go **red** on the broken build — Duhem drives the
real front and fails `AC-2` while `/health` still reports healthy — then
**green** once the fix ships:

```text
$ duhem run          # front is broken; /health still reports healthy
fail
  AC-2::AC-2.1:
    fail  $steps.front.outputs.body.title == "Acme Dashboard"
        (actual "Welcome to nginx!", expected "Acme Dashboard")

# fix shipped — re-run the same gate
$ duhem run
pass
```

## Files

| File | What it is |
|------|------------|
| `app.js` | The "app": `/health` always healthy; `/` serves the real app only when `APP_FIXED=1`, else a default page (both `200`). |
| `duhem.yml` | The Verification Definition. `AC-1` checks health; `AC-2` checks the front actually serves the app. |
| `run.sh` | Starts the app and runs the gate twice — broken, then fixed. The source the README demo is captured from. |
| `render-svg.mjs` | Renders that real output into the animated `demo.svg` (no deps: `node render-svg.mjs`). |

The README animation shows real output from these two runs; only the
pacing is synthesized.
