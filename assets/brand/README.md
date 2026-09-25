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

`docs/duhem-brand.md` §7 states the lockup rule as "mark height = wordmark cap height" and "gap = mark frame thickness × (wordmark cap height / mark canvas size)". The generator implements this as: the mark's *visual* span (28 of its 32-unit canvas, matching the bars' extent) is scaled to equal the wordmark's cap height in pixels, and the gap between mark and wordmark equals the mark's frame thickness (4 units) at that same scale. Vertical alignment centers the mark's geometric middle on the wordmark's ink (visual) center rather than its baseline, since "Duhem" has no descenders and baseline-centering would sit the mark visibly low. This produces the same qualitative result as the doc's own illustrative lockup SVG in §7, computed exactly rather than eyeballed.
