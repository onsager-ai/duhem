#!/usr/bin/env node
// Generator for the Duhem brand asset kit.
//
// Emits every file listed in docs/duhem-brand.md Appendix A (plus the
// light/dark README variants and the social-preview image) from the
// geometry constants below. Re-running this script must produce
// byte-identical output — see `npm run generate` and the determinism
// check documented in assets/brand/README.md.
//
// Usage: node generate.mjs (run from assets/brand/, after `npm install`)

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import * as wawoff2 from 'wawoff2';
import opentype from 'opentype.js';
import { Resvg } from '@resvg/resvg-js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = __dirname;

// ---------------------------------------------------------------------
// 1. Mark geometry — docs/duhem-brand.md §1. Load-bearing: don't change
//    without a spec update to the brand doc.
// ---------------------------------------------------------------------

const CANVAS = 32;
const FRAME_THICKNESS = 4;
const MARK_RECTS = [
  { x: 2, y: 2, w: 28, h: 4 }, // top bar
  { x: 2, y: 26, w: 28, h: 4 }, // bottom bar
  { x: 2, y: 6, w: 4, h: 20 }, // left bar
  { x: 26, y: 6, w: 4, h: 20 }, // right bar
  { x: 11, y: 11, w: 10, h: 10 }, // center square
];
// Visual span of the mark within its 32x32 canvas (bars run 2..30).
const MARK_VISUAL_SPAN = 28;

const ACCENT_NAVY = '#1E3A5F';

function markRectsSvg(fill) {
  const fillAttr = fill === 'currentColor' ? '' : ` fill="${fill}"`;
  return MARK_RECTS.map(
    (r) => `<rect x="${r.x}" y="${r.y}" width="${r.w}" height="${r.h}"${fillAttr}/>`
  ).join('\n    ');
}

function markGroup(fill, transform) {
  const t = transform ? ` transform="${transform}"` : '';
  const groupFill = fill === 'currentColor' ? ' fill="currentColor"' : '';
  return `<g${groupFill}${t}>\n    ${markRectsSvg(fill === 'currentColor' ? 'currentColor' : fill)}\n  </g>`;
}

// ---------------------------------------------------------------------
// 2. Wordmark — outlined Inter 400 glyph paths (docs/duhem-brand.md §6).
//    Outlined so the SVGs render identically without Inter installed
//    (GitHub README rendering has no Inter available).
// ---------------------------------------------------------------------

const TRACKING_EM = -0.005; // -0.5% tracking at display size, per §6.
const WORDMARK_TEXT = 'Duhem';

async function loadInter() {
  const woff2Path = join(
    OUT_DIR,
    'node_modules/@fontsource/inter/files/inter-latin-400-normal.woff2'
  );
  const woff2Buf = readFileSync(woff2Path);
  const ttfBuf = Buffer.from(await wawoff2.decompress(woff2Buf));
  const arrayBuffer = ttfBuf.buffer.slice(
    ttfBuf.byteOffset,
    ttfBuf.byteOffset + ttfBuf.byteLength
  );
  return opentype.parse(arrayBuffer);
}

// Build outlined glyph paths for `text` at `fontSize`, applying `trackingEm`
// tracking between glyphs. Returns per-glyph path `d` strings (each already
// positioned along the baseline starting at x=0), the combined ink bbox, and
// the total advance width — all in the same px units as `fontSize`.
function buildTextGeometry(font, text, fontSize, trackingEm = 0) {
  const scale = fontSize / font.unitsPerEm;
  let x = 0;
  let prevGlyph = null;
  const glyphPaths = [];
  let bbox = null;
  for (const ch of text) {
    const glyph = font.charToGlyph(ch);
    if (prevGlyph) {
      x += font.getKerningValue(prevGlyph, glyph) * scale;
    }
    const path = glyph.getPath(x, 0, fontSize);
    const d = path.toPathData(4);
    if (d) glyphPaths.push(d);
    const b = path.getBoundingBox();
    if (Number.isFinite(b.x1)) {
      bbox = bbox
        ? {
            x1: Math.min(bbox.x1, b.x1),
            y1: Math.min(bbox.y1, b.y1),
            x2: Math.max(bbox.x2, b.x2),
            y2: Math.max(bbox.y2, b.y2),
          }
        : { ...b };
    }
    x += glyph.advanceWidth * scale + trackingEm * fontSize;
    prevGlyph = glyph;
  }
  return { d: glyphPaths.join(' '), bbox, width: x };
}

function round(n, decimals = 3) {
  const f = 10 ** decimals;
  return Math.round(n * f) / f;
}

// ---------------------------------------------------------------------
// 3. SVG assembly helpers
// ---------------------------------------------------------------------

function svgDoc({ viewBox, width, height, title, body, extraAttrs = '' }) {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" width="${width}" height="${height}" role="img" aria-label="${title}"${extraAttrs}>
  <title>${title}</title>
  ${body}
</svg>
`;
}

// Mark-only SVG (currentColor), 32x32 canvas — doubles as both `duhem.svg`
// (master vector) and `duhem-mark-only.svg` (Appendix A lists them as
// separate use cases; the content is identical, currentColor, 32x32).
function markOnlySvg() {
  return svgDoc({
    viewBox: `0 0 ${CANVAS} ${CANVAS}`,
    width: CANVAS,
    height: CANVAS,
    title: 'Duhem',
    body: `<g fill="currentColor">
    ${markRectsSvg('currentColor')}
  </g>`,
  });
}

function wordmarkGeometry(font, fontSize) {
  return buildTextGeometry(font, WORDMARK_TEXT, fontSize, TRACKING_EM);
}

function wordmarkOnlySvg(font) {
  const FS = 64;
  const { d, bbox } = wordmarkGeometry(font, FS);
  const pad = FS * 0.06;
  const w = round(bbox.x2 - bbox.x1 + pad * 2);
  const h = round(bbox.y2 - bbox.y1 + pad * 2);
  const tx = round(pad - bbox.x1);
  const ty = round(pad - bbox.y1);
  return svgDoc({
    viewBox: `0 0 ${w} ${h}`,
    width: round(w / 2),
    height: round(h / 2),
    title: 'Duhem',
    body: `<g fill="currentColor" transform="translate(${tx} ${ty})">
    <path d="${d}"/>
  </g>`,
  });
}

// Shared horizontal-lockup layout math (docs/duhem-brand.md §7):
//   - mark visual height (28 units) = wordmark cap height
//   - gap between mark and wordmark = mark frame thickness, scaled by the
//     same mark-units-to-px factor used to size the mark
//   - vertical alignment: mark's geometric center matches the wordmark's
//     ink (visual) center, not its baseline
function horizontalLockupLayout(font, fontSize) {
  const { d, bbox, width } = wordmarkGeometry(font, fontSize);
  const capHeightPx = -bbox.y1; // ink top is the cap/ascender line for "Duhem"
  const markScale = capHeightPx / MARK_VISUAL_SPAN;
  const markCanvasPx = CANVAS * markScale;
  const gapPx = FRAME_THICKNESS * markScale;

  const markCenterY = markCanvasPx / 2; // mark box is symmetric top/bottom
  const inkCenterYFromBaseline = (bbox.y1 + bbox.y2) / 2;
  const baselineY = markCenterY - inkCenterYFromBaseline;

  const textLeft = markCanvasPx + gapPx;
  const textRight = textLeft + width;

  const top = Math.min(0, baselineY + bbox.y1);
  const bottom = Math.max(markCanvasPx, baselineY + bbox.y2);
  const pad = markScale * 2; // mirrors the mark's own 2-unit inset

  const minX = -pad;
  const minY = top - pad;
  const w = textRight - minX + pad;
  const h = bottom - minY + pad;

  return { d, markScale, baselineY, textLeft, minX, minY, w, h };
}

function horizontalLockupSvg(font, { fill = 'currentColor', title = 'Duhem' } = {}) {
  const FS = 64;
  const { d, markScale, baselineY, textLeft, minX, minY, w, h } = horizontalLockupLayout(
    font,
    FS
  );
  const markFillAttr = fill === 'currentColor' ? ' fill="currentColor"' : ` fill="${fill}"`;
  const textFillAttr = fill === 'currentColor' ? ' fill="currentColor"' : ` fill="${fill}"`;
  const tx = round(-minX);
  const ty = round(-minY);
  return svgDoc({
    viewBox: `0 0 ${round(w)} ${round(h)}`,
    width: round(w / 2),
    height: round(h / 2),
    title,
    body: `<g transform="translate(${tx} ${ty})">
    <g${markFillAttr} transform="scale(${round(markScale, 5)})">
      ${markRectsSvg(fill)}
    </g>
    <path${textFillAttr} transform="translate(${round(textLeft)} ${round(baselineY)})" d="${d}"/>
  </g>`,
  });
}

function verticalLockupSvg(font, { fill = 'currentColor', title = 'Duhem' } = {}) {
  const FS = 64;
  // Mark sized against the wordmark cap height, same rule as the
  // horizontal lockup, then stacked with a frame-thickness gap.
  const { bbox, d, width } = wordmarkGeometry(font, FS);
  const capHeightPx = -bbox.y1;
  const markScale = capHeightPx / MARK_VISUAL_SPAN;
  const markCanvasPx = CANVAS * markScale;
  const gapPx = FRAME_THICKNESS * markScale;

  const inkWidth = bbox.x2 - bbox.x1;
  const markCenterX = markCanvasPx / 2;
  const textLeft = markCenterX - inkWidth / 2 - bbox.x1;
  const textTop = markCanvasPx + gapPx;
  const baselineY = textTop - bbox.y1;

  const pad = markScale * 2;
  const contentW = Math.max(markCanvasPx, inkWidth);
  const minX = Math.min(0, textLeft + bbox.x1) - pad;
  const maxX = Math.max(markCanvasPx, textLeft + bbox.x2) + pad;
  const maxY = baselineY + bbox.y2 + pad;
  const minY = -pad;

  const w = maxX - minX;
  const h = maxY - minY;
  const markFillAttr = fill === 'currentColor' ? ' fill="currentColor"' : ` fill="${fill}"`;
  const textFillAttr = fill === 'currentColor' ? ' fill="currentColor"' : ` fill="${fill}"`;

  return svgDoc({
    viewBox: `0 0 ${round(w)} ${round(h)}`,
    width: round(w / 2),
    height: round(h / 2),
    title,
    body: `<g transform="translate(${round(-minX)} ${round(-minY)})">
    <g${markFillAttr} transform="translate(${round(markCenterX - markCanvasPx / 2)} 0) scale(${round(markScale, 5)})">
      ${markRectsSvg(fill)}
    </g>
    <path${textFillAttr} transform="translate(${round(textLeft)} ${round(baselineY)})" d="${d}"/>
  </g>`,
  });
}

// ---------------------------------------------------------------------
// 4. Raster (PNG / ICO) generation
// ---------------------------------------------------------------------

function rasterMarkSvg(fill, sizePx) {
  return svgDoc({
    viewBox: `0 0 ${CANVAS} ${CANVAS}`,
    width: sizePx,
    height: sizePx,
    title: 'Duhem',
    body: `<g fill="${fill}">
    ${markRectsSvg(fill)}
  </g>`,
  });
}

function renderPng(svgString, sizePx) {
  const resvg = new Resvg(svgString, {
    fitTo: { mode: 'width', value: sizePx },
    background: 'rgba(0,0,0,0)',
  });
  return resvg.render().asPng();
}

function writeIco(outPath, entries) {
  // entries: [{ size, png: Buffer }], ascending or any order.
  const count = entries.length;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: 1 = icon
  header.writeUInt16LE(count, 4);

  const dirEntries = [];
  const imageBuffers = [];
  let offset = 6 + 16 * count;
  for (const { size, png } of entries) {
    const entry = Buffer.alloc(16);
    entry.writeUInt8(size >= 256 ? 0 : size, 0); // width
    entry.writeUInt8(size >= 256 ? 0 : size, 1); // height
    entry.writeUInt8(0, 2); // color count (0 = no palette)
    entry.writeUInt8(0, 3); // reserved
    entry.writeUInt16LE(1, 4); // color planes
    entry.writeUInt16LE(32, 6); // bits per pixel
    entry.writeUInt32LE(png.length, 8); // size of image data
    entry.writeUInt32LE(offset, 12); // offset of image data
    dirEntries.push(entry);
    imageBuffers.push(png);
    offset += png.length;
  }
  writeFileSync(outPath, Buffer.concat([header, ...dirEntries, ...imageBuffers]));
}

function roundedRectPath(x, y, w, h, r) {
  return `M${x + r},${y} H${x + w - r} A${r},${r} 0 0 1 ${x + w},${y + r} V${y + h - r} A${r},${r} 0 0 1 ${x + w - r},${y + h} H${x + r} A${r},${r} 0 0 1 ${x},${y + h - r} V${y + r} A${r},${r} 0 0 1 ${x + r},${y}Z`;
}

function appIconSvg() {
  const SIZE = 1024;
  const plateRadius = round(SIZE * 0.223);
  const markPx = round(SIZE * 0.6);
  const offset = round((SIZE - markPx) / 2);
  const markScale = round(markPx / CANVAS, 6);
  return svgDoc({
    viewBox: `0 0 ${SIZE} ${SIZE}`,
    width: SIZE,
    height: SIZE,
    title: 'Duhem app icon',
    body: `<path fill="${ACCENT_NAVY}" d="${roundedRectPath(0, 0, SIZE, SIZE, plateRadius)}"/>
  <g fill="#FFFFFF" transform="translate(${offset} ${offset}) scale(${markScale})">
    ${markRectsSvg('#FFFFFF')}
  </g>`,
  });
}

// ---------------------------------------------------------------------
// 5. Social preview (1280x640)
// ---------------------------------------------------------------------

const TAGLINE = 'Holistic verification for AI-built software.';

function socialPreviewSvg(font) {
  const W = 1280;
  const H = 640;

  // Vertical lockup, scaled up, centered in the safe area.
  const lockupFS = 176;
  const { bbox: wmBbox, d: wmD, width: wmWidth } = wordmarkGeometry(font, lockupFS);
  const capHeightPx = -wmBbox.y1;
  const markScale = capHeightPx / MARK_VISUAL_SPAN;
  const markCanvasPx = CANVAS * markScale;
  const gapPx = FRAME_THICKNESS * markScale;

  const inkWidth = wmBbox.x2 - wmBbox.x1;
  const lockupWidth = Math.max(markCanvasPx, inkWidth);
  const lockupHeight = markCanvasPx + gapPx + (wmBbox.y2 - wmBbox.y1);

  // Tagline, sized to fit comfortably inside the safe area.
  let taglineFS = 44;
  let tagline = buildTextGeometry(font, TAGLINE, taglineFS, 0);
  const maxTaglineWidth = W * 0.72;
  if (tagline.width > maxTaglineWidth) {
    taglineFS = round(taglineFS * (maxTaglineWidth / tagline.width), 2);
    tagline = buildTextGeometry(font, TAGLINE, taglineFS, 0);
  }

  const tagGap = 56; // gap between lockup and tagline
  const blockHeight = lockupHeight + tagGap + (tagline.bbox.y2 - tagline.bbox.y1);
  const blockTop = (H - blockHeight) / 2;

  const markX = (W - lockupWidth) / 2 - wmBbox.x1 < 0 ? (W - lockupWidth) / 2 : (W - markCanvasPx) / 2;
  const markLeft = (W - markCanvasPx) / 2;
  const markTop = blockTop;

  const textLeft = (W - inkWidth) / 2 - wmBbox.x1;
  const textBaselineY = markTop + markCanvasPx + gapPx - wmBbox.y1;

  const tagInkWidth = tagline.bbox.x2 - tagline.bbox.x1;
  const tagLeft = (W - tagInkWidth) / 2 - tagline.bbox.x1;
  const tagBaselineY = markTop + markCanvasPx + gapPx + (wmBbox.y2 - wmBbox.y1) + tagGap - tagline.bbox.y1;

  return svgDoc({
    viewBox: `0 0 ${W} ${H}`,
    width: W,
    height: H,
    title: 'Duhem — Holistic verification for AI-built software.',
    extraAttrs: '',
    body: `<rect x="0" y="0" width="${W}" height="${H}" fill="#FFFFFF"/>
  <g fill="#000000" transform="translate(${round(markLeft)} ${round(markTop)}) scale(${round(markScale, 5)})">
    ${markRectsSvg('#000000')}
  </g>
  <path fill="#000000" transform="translate(${round(textLeft)} ${round(textBaselineY)})" d="${wmD}"/>
  <path fill="${ACCENT_NAVY}" transform="translate(${round(tagLeft)} ${round(tagBaselineY)})" d="${tagline.d}"/>`,
  });
}

// ---------------------------------------------------------------------
// 6. Main
// ---------------------------------------------------------------------

async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  const font = await loadInter();

  const write = (name, content) => writeFileSync(join(OUT_DIR, name), content);

  // SVGs — currentColor, for in-repo / doc / app use.
  write('duhem.svg', markOnlySvg());
  write('duhem-mark-only.svg', markOnlySvg());
  write('duhem-wordmark.svg', wordmarkOnlySvg(font));
  write('duhem-lockup.svg', horizontalLockupSvg(font));
  write('duhem-lockup-vertical.svg', verticalLockupSvg(font));

  // Fixed-color horizontal lockup for the README <picture> (GitHub can't
  // resolve currentColor in README images).
  write(
    'duhem-lockup-light.svg',
    horizontalLockupSvg(font, { fill: '#000000', title: 'Duhem (for light backgrounds)' })
  );
  write(
    'duhem-lockup-dark.svg',
    horizontalLockupSvg(font, { fill: '#FFFFFF', title: 'Duhem (for dark backgrounds)' })
  );

  // PNGs — black mark on transparent background, standard favicon/icon sizes.
  const pngSizes = [16, 32, 48, 64, 192, 512];
  const icoPngs = [];
  for (const size of pngSizes) {
    const svg = rasterMarkSvg('#000000', size);
    const png = renderPng(svg, size);
    write(`duhem-${size}.png`, png);
    if ([16, 32, 48].includes(size)) icoPngs.push({ size, png });
  }
  writeIco(join(OUT_DIR, 'duhem-favicon.ico'), icoPngs);

  // App icon — mark on a solid navy rounded-square plate.
  write('duhem-app-icon.png', renderPng(appIconSvg(), 1024));

  // Social preview — 1280x640, vertical lockup + tagline.
  write('social-preview.png', renderPng(socialPreviewSvg(font), 1280));

  console.log('Generated brand assets in', OUT_DIR);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
