# Duhem brand asset kit

Generated files for `docs/duhem-brand.md` Appendix A. The source of truth is `generate.mjs`, which derives every file below from the mark geometry (`docs/duhem-brand.md` §1) and an outlined Inter 400 wordmark (§6) — no hand-edited SVGs, no Inter installation required to render them.

## Regenerating

```sh
cd assets/brand
npm install   # once, or after a dependency bump
npm run generate
```

This overwrites every generated file in this directory in place. The script is deterministic: running it twice produces byte-identical output (verified via `sha256sum * | sort` before/after during development).

## What's here

| File | Description |
|---|---|
| `duhem.svg`, `duhem-mark-only.svg` | Master mark, `currentColor`, 32×32 viewBox. Identical content — two names for the two use cases in Appendix A. |
| `duhem-wordmark.svg` | Wordmark only, outlined Inter 400 paths, `currentColor`. |
| `duhem-lockup.svg` | Horizontal lockup (mark + wordmark), `currentColor`. |
| `duhem-lockup-vertical.svg` | Vertical lockup (mark above wordmark), `currentColor`. |
| `duhem-lockup-light.svg` | Horizontal lockup, fixed black fill, transparent background — for READMEs/pages rendered on a light background (GitHub can't resolve `currentColor` in README `<img>`/`<picture>` sources). |
| `duhem-lockup-dark.svg` | Horizontal lockup, fixed white fill, transparent background — dark-background counterpart. |
| `duhem-{16,32,48,64,192,512}.png` | Mark-only, black fill, transparent background, at each size. |
| `duhem-favicon.ico` | Multi-size ICO (16/32/48) containing PNG-format icon entries. |
| `duhem-app-icon.png` | 1024×1024 app icon: mark in white at ~60% plate width, centered on a solid navy (`#1E3A5F`) rounded-square plate. |
| `social-preview.png` | 1280×640 GitHub social-preview image: vertical lockup + the README tagline, monochrome with the navy accent, content kept inside the crop-safe center. |

## Why these dependencies

- **`opentype.js`** parses the Inter TTF and emits glyph outlines as SVG path data, so the wordmark in every SVG is baked-in vector paths rather than a `<text>` element — it renders identically with or without Inter installed (GitHub's README renderer has no Inter).
- **`wawoff2`** decompresses the WOFF2 files `@fontsource/inter` ships (opentype.js doesn't read WOFF2 directly) into a TTF `opentype.js` can parse.
- **`@fontsource/inter`** is the pinned, OFL-licensed source of the actual Inter font bytes (only the woff2 regular-weight file is used; nothing under `node_modules` is committed or shipped).
- **`@resvg/resvg-js`** rasterizes the generated SVGs to PNG deterministically (no headless browser, no network at render time).

All four are `devDependencies` of this directory's own `package.json`, isolated from the product crates and the repo root — they're build-time tooling for this generator only.

## Layout notes (an interpretation, not a literal transcription of §7)

`docs/duhem-brand.md` §7 states the lockup rule as "mark height = wordmark cap height" and "gap = mark frame thickness × (wordmark cap height / mark canvas size)". The generator implements this as: the mark's *visual* span (28 of its 32-unit canvas, matching the bars' extent) is scaled to equal the wordmark's cap height in pixels, and the gap between mark and wordmark equals the mark's frame thickness (4 units) at that same scale. Vertical alignment centers the mark's geometric middle on the wordmark's ink (visual) center rather than its baseline, since "Duhem" has no descenders and baseline-centering would sit the mark visibly low. This produces the same qualitative result as the doc's own illustrative lockup SVG in §7, computed exactly rather than eyeballed. This §7 rule is for the *horizontal* lockup only (`duhem-lockup{,-light,-dark}.svg`).

### Vertical lockup and social preview: mark size

§7 gives no sizing rule for the vertical variant (mark above wordmark). Applying the horizontal rule's "mark height = wordmark cap height" there reads undersized, because the mark then sits over a wordmark that also has full descender/ascender width below and beside it, not just a single cap-height line. `duhem-lockup-vertical.svg` instead scales the mark to `VERTICAL_MARK_CAP_RATIO = 1.5` × the wordmark cap height — a deliberate, documented choice, not derived from the brand doc. `social-preview.png` reuses the same intent (its wordmark font size and mark scale are picked so the mark lands close to that same ~1.5× ratio) but is constrained by the integer-pixel rule below, so it isn't an exact 1.5×.

### Raster outputs: integer px-per-grid-unit + `shape-rendering="crispEdges"`

The mark is five axis-aligned rects on a 32-unit grid. When a raster output's scale factor isn't a whole number of pixels per grid unit, a rect edge lands on a fractional pixel and the rasterizer anti-aliases it — for a straight edge this doesn't blend into a diagonal, it just leaves a partial-coverage (mid-grey, e.g. 50% grey at a half-covered pixel) row or column sitting on what should be a crisp black/white edge. This showed up first in `social-preview.png` at its old ≈4.57 px/unit scale (a 127,127,127 grey line at the bottom-bar seam) and is also present in `duhem-48.png` at its native 1.5 px/unit scale (the center square's edges, at unit 11 × 1.5 = 16.5px).

Two fixes, both applied:

1. Where a script constant controls the scale (`social-preview.png`, `duhem-app-icon.png`), the constant is chosen so px-per-grid-unit is a whole number: `SOCIAL_MARK_PX_PER_UNIT = 7` for the social preview (`markCanvasPx` = 224, `gapPx` = 28, mark placement rounded to the nearest px), and an integer scale for the app icon (see below).
2. Every raster mark group (`rasterMarkSvg` for the `duhem-*.png` sizes and the ICO source images, `appIconSvg`, and the social-preview's mark group) carries `shape-rendering="crispEdges"`, which tells the SVG rasterizer to snap axis-aligned edges to the pixel grid instead of anti-aliasing them — a safety net for any size (like the 16/32/64/192/512 PNGs) that isn't hand-tuned to an exact integer scale. Verified pixel-exact (0 partial-alpha / mid-grey pixels) across all six PNG sizes and the social preview's mark region after the fix, by decoding the actual committed PNGs and scanning for non-pure-black/white or partial-alpha pixels.

The wordmark and tagline (glyph outline paths, not rects) are unaffected by and don't need `crispEdges` — normal anti-aliasing on curved/diagonal type is expected and correct.

### App icon: ~60% of plate width means the *visual* mark, not its canvas

§9 says the mark should occupy "~60% of plate width". The mark's own 32-unit canvas has a 2-unit empty inset on each side (the bars run 2..30), so scaling the *whole 32-unit canvas* to 60% of the plate under-sizes the *visible* mark to 60% × (28/32) ≈ 52.5% of the plate — which is what an earlier version of this script did (528/1024 ≈ 52%, i.e. `markPx = round(1024 * 0.6)` scaled the full canvas). The fix scales the mark so its 28-unit *visual* span, not its 32-unit canvas, is the one that lands near 60% of the plate, on an integer px-per-grid-unit: `APP_ICON_TARGET_VISUAL_RATIO = 0.6` resolves to a 22px/unit scale (`Math.round(0.6 × 1024 / 28) = 22`), giving a 704px mark canvas, a 616px visual span, and a 616/1024 ≈ 60.2% ratio — on the nose, with every edge (canvas and visual span alike) on a whole pixel. The plate stays the navy `#1E3A5F` accent (§5), matching the ratio the dashboard's own apple-touch-icon (a sibling branch, Phase 2) independently landed on (≈62%).
