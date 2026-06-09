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

let browser = null
const contexts = new Map() // id -> BrowserContext
const pages = new Map() // id -> Page
let nextHandle = 1

function send(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n')
}

function page(req) {
  const p = pages.get(req.pageId)
  if (!p) throw new Error(`unknown pageId: ${req.pageId}`)
  return p
}

async function dispatch(req) {
  switch (req.op) {
    case 'launch':
      browser = await chromium.launch({ headless: req.headless !== false })
      return null

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
    send({ id: req.id, ok: false, error: e && e.message ? e.message : String(e) })
  }
})

// If stdin closes (parent dropped the connection), exit cleanly.
rl.on('close', () => process.exit(0))
