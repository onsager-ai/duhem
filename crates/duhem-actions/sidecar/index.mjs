// @ts-check
// Duhem Playwright sidecar — onsager-ai/duhem#71.
//
// A thin, Duhem-owned bridge between the Rust `duhem-actions` crate
// and the official (maintained, Apple-Silicon-capable) Playwright Node
// package. It speaks newline-delimited JSON-RPC over stdio: one request
// object per line on stdin, one response object per line on stdout.
//
// Protocol:
//   request:  { "id": <u64>, "op": "<name>", ...params }
//   response: { "id": <u64>, "ok": true,  "result": <any|null> }
//          |  { "id": <u64>, "ok": false, "error": "<message>" }
//
// The runtime issues one request at a time and waits for its response
// (checks and steps run sequentially), so no request multiplexing is
// needed here. The bridge intentionally exposes ONLY the operations
// `duhem-actions` uses — it is not a general Playwright RPC surface.
//
// Locator semantics are owned by Rust (`to_selector`): selectors arrive
// as Playwright selector-engine strings (e.g. `role=button[name="X"]`)
// and are handed to `page.locator(selector)` verbatim. Playwright's own
// ARIA-role / accessible-name / auto-wait engine does the rest — that
// fidelity is the whole reason this is a sidecar and not a CDP rewrite.

import { chromium } from 'playwright'
import readline from 'node:readline'
import { existsSync, readdirSync } from 'node:fs'
import { execSync } from 'node:child_process'
import { join } from 'node:path'
import os from 'node:os'

/**
 * Best-effort discovery of an already-installed Chromium/Chrome, used
 * when Playwright's own bundled-browser launch fails (e.g. on an OS
 * Playwright ships no prebuilt browser for, or where the cached browser
 * revision doesn't match this Playwright). Mirrors what an operator
 * would otherwise pass via DUHEM_BROWSER_EXECUTABLE — checked here so
 * the common case needs no manual configuration. Returns an absolute
 * path or undefined.
 * @returns {string | undefined}
 */
function discoverChromium() {
  // 1. Any Chromium revision in a Playwright browser cache. Prefer the
  //    full `chromium-<rev>` build, and the highest revision available.
  const cacheDirs = [
    process.env.PLAYWRIGHT_BROWSERS_PATH,
    join(os.homedir(), '.cache', 'ms-playwright'), // linux
    join(os.homedir(), 'Library', 'Caches', 'ms-playwright'), // macOS
    join(os.homedir(), 'AppData', 'Local', 'ms-playwright'), // windows
  ].filter(Boolean)
  const subpaths = [
    ['chrome-linux64', 'chrome'],
    ['chrome-linux', 'chrome'],
    ['chrome-mac', 'Chromium.app', 'Contents', 'MacOS', 'Chromium'],
    ['chrome-win', 'chrome.exe'],
  ]
  for (const dir of cacheDirs) {
    if (!dir || !existsSync(dir)) continue
    let entries = []
    try {
      entries = readdirSync(dir)
    } catch {
      continue
    }
    const revs = entries
      .filter((e) => e.startsWith('chromium-'))
      .sort()
      .reverse()
    for (const rev of revs) {
      for (const sp of subpaths) {
        const p = join(dir, rev, ...sp)
        if (existsSync(p)) return p
      }
    }
  }
  // 2. A system browser on PATH.
  for (const name of [
    'google-chrome',
    'google-chrome-stable',
    'chromium',
    'chromium-browser',
    'microsoft-edge',
  ]) {
    try {
      const p = execSync(`command -v ${name} 2>/dev/null`, { shell: '/bin/sh' })
        .toString()
        .trim()
      if (p && existsSync(p)) return p
    } catch {
      // not on PATH; try the next candidate
    }
  }
  return undefined
}

/**
 * @typedef {import('playwright').Browser} Browser
 * @typedef {import('playwright').BrowserContext} BrowserContext
 * @typedef {import('playwright').Page} Page
 */

/**
 * One recorded response, mirrored into the Rust `NetworkEvent` struct
 * in `browser.rs`. Bodies are base64 so raw bytes survive the JSON hop;
 * `observe.rs` owns UTF-8-lossy text + JSON parsing.
 *
 * @typedef {Object} NetworkRecord
 * @property {string} method
 * @property {string} url
 * @property {number} status
 * @property {Record<string, string>} requestHeaders
 * @property {string | null} requestBodyBase64
 * @property {Record<string, string>} responseHeaders
 * @property {string | null} bodyBase64
 * @property {string | null} bodyError
 */

/** @type {Browser | null} */
let browser = null
/** @type {Map<string, BrowserContext>} */
const contexts = new Map()
/** @type {Map<string, Page>} */
const pages = new Map()
/** @type {Map<string, NetworkRecord[]>} */
const networkBuffers = new Map()
let nextHandle = 1

/** @param {unknown} e */
function errMsg(e) {
  return e instanceof Error ? e.message : String(e)
}

/** @param {{ id?: number, ok: boolean, result?: unknown, error?: string }} obj */
function send(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n')
}

/**
 * @param {{ pageId: string }} req
 * @returns {Page}
 */
function page(req) {
  const p = pages.get(req.pageId)
  if (!p) throw new Error(`unknown pageId: ${req.pageId}`)
  return p
}

// Record every response on the page into `buf` for `api/observe`
// (onsager-ai/duhem#72). Bodies are read eagerly and base64-encoded so
// the Rust side gets the raw bytes (UTF-8-lossy text + JSON parse are
// owned by `observe.rs`, byte-for-byte with the pre-#71 implementation).
// A body read failure is captured as `bodyError` and only propagated by
// Rust when the event is the matched one — mirroring the old
// collect-on-match semantics, so unrelated failures (redirects, aborted
// requests) never break an observe that matches a different response.
/**
 * @param {Page} p
 * @param {NetworkRecord[]} buf
 */
function attachNetworkRecorder(p, buf) {
  p.on('response', async (response) => {
    const request = response.request()
    /** @type {NetworkRecord} */
    const rec = {
      method: request.method(),
      url: request.url(),
      status: response.status(),
      // Playwright lowercases header names already; `observe.rs`
      // re-lowercases defensively, so either casing is safe here.
      requestHeaders: request.headers(),
      requestBodyBase64: null,
      responseHeaders: response.headers(),
      bodyBase64: null,
      bodyError: null,
    }
    const pd = request.postDataBuffer()
    if (pd) rec.requestBodyBase64 = pd.toString('base64')
    try {
      const body = await response.body()
      rec.bodyBase64 = body.toString('base64')
    } catch (e) {
      rec.bodyError = errMsg(e)
    }
    buf.push(rec)
  })
}

/** @param {any} req */
async function dispatch(req) {
  switch (req.op) {
    case 'launch': {
      // By default Playwright launches its own bundled Chromium. On a
      // host where that download is unavailable (e.g. an OS Playwright
      // ships no prebuilt browser for), an operator can point the
      // sidecar at a system browser via env — without these set, the
      // behavior is unchanged:
      //   DUHEM_BROWSER_EXECUTABLE — path to a Chromium/Chrome binary
      //   DUHEM_BROWSER_CHANNEL    — a Playwright channel (e.g. "chrome")
      //   DUHEM_BROWSER_ARGS       — extra launch args, space-separated
      //     (e.g. "--no-sandbox" when running inside a container)
      const executablePath = process.env.DUHEM_BROWSER_EXECUTABLE || undefined
      const channel = process.env.DUHEM_BROWSER_CHANNEL || undefined
      const extraArgs = (process.env.DUHEM_BROWSER_ARGS || '')
        .split(/\s+/)
        .filter(Boolean)
      const baseOpts = {
        headless: req.headless !== false,
        executablePath,
        channel,
        args: extraArgs,
      }
      try {
        browser = await chromium.launch(baseOpts)
      } catch (e) {
        // The bundled-browser launch failed. If the operator didn't
        // pin an executable/channel, try an already-installed Chromium
        // before giving up — this is what unblocks a host where
        // `playwright install` can't fetch a browser (unsupported OS,
        // or a cached revision that doesn't match this Playwright).
        if (executablePath || channel) throw e
        const discovered = discoverChromium()
        if (!discovered) {
          throw new Error(
            `${errMsg(e)} — and no existing Chromium was found to fall back to. ` +
              `Install one (e.g. \`npx playwright install chromium\`) or set ` +
              `DUHEM_BROWSER_EXECUTABLE=/path/to/chrome to use an existing browser.`,
          )
        }
        process.stderr.write(
          `[duhem-sidecar] bundled browser launch failed; falling back to discovered Chromium at ${discovered}\n`,
        )
        browser = await chromium.launch({ ...baseOpts, executablePath: discovered })
      }
      return null
    }

    case 'newContext': {
      if (!browser) throw new Error('newContext before launch')
      const ctx = await browser.newContext()
      const id = 'c' + nextHandle++
      contexts.set(id, ctx)
      return { contextId: id }
    }

    case 'newPage': {
      const ctx = contexts.get(req.contextId)
      if (!ctx) throw new Error(`unknown contextId: ${req.contextId}`)
      const p = await ctx.newPage()
      const id = 'p' + nextHandle++
      pages.set(id, p)
      /** @type {NetworkRecord[]} */
      const buf = []
      networkBuffers.set(id, buf)
      attachNetworkRecorder(p, buf)
      return { pageId: id }
    }

    case 'goto':
      await page(req).goto(req.url, { timeout: req.timeoutMs })
      return null

    case 'click':
      await page(req).click(req.selector, { timeout: req.timeoutMs })
      return null

    case 'fill':
      await page(req).fill(req.selector, req.text, { timeout: req.timeoutMs })
      return null

    case 'type':
      // Append semantics (no clear), matching the old `ui/type clear:false`
      // path. `pressSequentially` is the non-deprecated equivalent of the
      // old `page.type`; it auto-waits for actionability.
      await page(req)
        .locator(req.selector)
        .pressSequentially(req.text, { timeout: req.timeoutMs })
      return null

    case 'selectOption': {
      const by = req.by
      let option
      if (by.value !== undefined) option = { value: by.value }
      else if (by.label !== undefined) option = { label: by.label }
      else option = { index: by.index }
      await page(req).selectOption(req.selector, option, { timeout: req.timeoutMs })
      return null
    }

    case 'waitForSelector':
      // Resolves on reaching `state`; throws TimeoutError otherwise. The
      // Rust side maps the throw (message contains "Timeout") to
      // `satisfied: false`, never to a hard error.
      await page(req).waitForSelector(req.selector, {
        state: req.state,
        timeout: req.timeoutMs,
      })
      return null

    case 'count':
      return await page(req).locator(req.selector).count()

    case 'url':
      return page(req).url()

    case 'eval':
      // `req.expr` is a JS expression string (e.g. `document.readyState`).
      return await page(req).evaluate(req.expr)

    case 'cookies':
      return await page(req).context().cookies()

    case 'pollNetwork': {
      // Return recorded responses from `cursor` onward plus the new
      // cursor (buffer length). `observe.rs` polls this within its
      // `within:` window. The buffer is per-page and the page is
      // per-check, so it only ever holds this check's own traffic.
      const buf = networkBuffers.get(req.pageId)
      if (!buf) throw new Error(`unknown pageId: ${req.pageId}`)
      const from = req.cursor || 0
      return { events: buf.slice(from), cursor: buf.length }
    }

    case 'closeContext': {
      const ctx = contexts.get(req.contextId)
      if (ctx) {
        await ctx.close()
        contexts.delete(req.contextId)
      }
      return null
    }

    case 'shutdown':
      if (browser) await browser.close().catch(() => {})
      send({ id: req.id, ok: true, result: null })
      process.exit(0)
      return null

    default:
      throw new Error(`unknown op: ${req.op}`)
  }
}

const rl = readline.createInterface({ input: process.stdin })
rl.on('line', async (line) => {
  const trimmed = line.trim()
  if (!trimmed) return
  let req
  try {
    req = JSON.parse(trimmed)
  } catch {
    return // ignore unparseable lines
  }
  try {
    const result = await dispatch(req)
    send({ id: req.id, ok: true, result: result ?? null })
  } catch (e) {
    send({ id: req.id, ok: false, error: errMsg(e) })
  }
})

// If stdin closes (parent dropped the connection), exit cleanly.
rl.on('close', () => process.exit(0))
