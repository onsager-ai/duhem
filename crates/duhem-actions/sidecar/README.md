# duhem-playwright-sidecar

The Node sidecar that drives **official Playwright** for `duhem-actions`'
`ui/*` actions (spec [#71](https://github.com/onsager-ai/duhem/issues/71)).
It speaks newline-delimited JSON-RPC over stdio; the Rust client lives in
`crates/duhem-actions/src/browser.rs`. See `index.mjs` for the protocol.

## Setup (once per host)

Requires **Node ≥ 20**.

```sh
npm ci                          # install deps from the committed lockfile
npx playwright install chromium # install the matching Chromium
```

`npm ci` installs the exact `playwright` pinned in `package-lock.json`;
the Chromium revision is therefore reproducible (this also covers the
intent of issue #13). `node_modules/` is gitignored — never vendored.

The Rust runtime locates this directory via `CARGO_MANIFEST_DIR`
(`crates/duhem-actions/sidecar`); override with `DUHEM_SIDECAR_DIR`, and
the Node binary with `DUHEM_NODE`.

## Using a system browser (when the bundled Chromium is unavailable)

By default the sidecar launches Playwright's own bundled Chromium. On a
host Playwright ships no prebuilt browser for, `npx playwright install
chromium` hard-refuses (e.g. `Playwright does not support chromium on
<os>`). Point the sidecar at a browser already on the host instead, via
env read at launch (spec [#82](https://github.com/onsager-ai/duhem/issues/82)):

- `DUHEM_BROWSER_EXECUTABLE` — absolute path to a Chromium/Chrome binary.
- `DUHEM_BROWSER_CHANNEL` — a Playwright channel (e.g. `chrome`).
- `DUHEM_BROWSER_ARGS` — extra launch args, space-separated (e.g.
  `--no-sandbox` inside a container).

Unset → unchanged behavior. The sidecar inherits the `duhem` process
env, so export these before `duhem run`. Example (snap Chromium in a
container):

```sh
export DUHEM_BROWSER_EXECUTABLE=/snap/bin/chromium
export DUHEM_BROWSER_ARGS="--no-sandbox --disable-setuid-sandbox --disable-dev-shm-usage"
duhem run <verification>.yml
```

### Auto-discovery fallback (spec [#105](https://github.com/onsager-ai/duhem/issues/105))

If `DUHEM_BROWSER_EXECUTABLE` / `DUHEM_BROWSER_CHANNEL` are **unset** and
the bundled-browser launch fails, the sidecar tries to find an
already-installed Chromium before giving up — so a fresh `duhem run`
works with no manual configuration on a host where `playwright install`
can't fetch a browser (unsupported OS, or a cached browser revision that
doesn't match this Playwright). It searches, in order:

1. any `chromium-<rev>` build in a Playwright browser cache
   (`PLAYWRIGHT_BROWSERS_PATH`, then the per-OS `ms-playwright` cache),
   preferring the highest revision;
2. a system browser on `PATH` (`google-chrome`, `chromium`,
   `chromium-browser`, `microsoft-edge`).

It logs the chosen binary to stderr (`falling back to discovered
Chromium at …`). Setting `DUHEM_BROWSER_EXECUTABLE` skips discovery and
pins your choice. If nothing is found, the launch error names both
`npx playwright install chromium` and the `DUHEM_BROWSER_EXECUTABLE`
override.

## Type-checking

`index.mjs` is plain ESM (Node runs it with no build step), but it is
fully type-checked: `// @ts-check` at the top plus `jsconfig.json`
(`checkJs`, `strict`) run TypeScript's checker over it against
Playwright's bundled `.d.ts` and `@types/node` (a dev-only dependency).
You get type errors and intellisense at authoring time with **zero build
step and zero new runtime dependency** — the run path stays `node
index.mjs`. To check it locally:

```sh
npm ci                  # installs @types/node (devDependency)
npx tsc -p jsconfig.json # no emit; reports type errors only
```

## Troubleshooting the Chromium download

`npx playwright install chromium` fetches a ~170 MiB build from
`cdn.playwright.dev`. On restricted networks this can stall:

- **Behind an HTTP proxy** (e.g. a local `127.0.0.1:7890`): the large
  CDN fetch may choke. Try **without** the proxy:
  `env -u http_proxy -u https_proxy -u all_proxy npx playwright install chromium`.
- **No direct CDN access:** the pinned Playwright version maps to a
  fixed Chromium revision. If you already have that revision cached
  (`~/Library/Caches/ms-playwright/chromium-<rev>` on macOS), no
  download happens. To find the revision a version needs, check
  `node_modules/playwright-core/browsers.json`.
- A `PLAYWRIGHT_DOWNLOAD_HOST` mirror can help, but note that mirrors do
  not always carry the newer "Chrome for Testing" builds.

This friction is **local-dev only** — CI runners have no such proxy.
