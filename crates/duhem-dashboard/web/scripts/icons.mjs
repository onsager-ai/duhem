#!/usr/bin/env node
// Emit the dashboard's favicon set into public/ (copied verbatim into
// dist/ by Vite, embedded into the binary by rust-embed, and written
// out by `duhem dashboard export`).
//
//   public/favicon.svg          mark, black on light tabs / white on dark
//   public/favicon-32.png       32×32 PNG fallback, transparent (brand §9)
//   public/apple-touch-icon.png 180×180, mark ~60% on a solid navy plate
//
// The mark is five axis-aligned rectangles on a 32-unit grid
// (docs/duhem-brand.md §1), so every size here uses an integer px/unit
// scale and the PNGs are rasterized exactly — no anti-aliasing, no
// image library. Node built-ins only; re-running reproduces the same
// pixels. Run: `npm run icons`.

import { writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";

const RECTS = [
  [2, 2, 28, 4],
  [2, 26, 28, 4],
  [2, 6, 4, 20],
  [26, 6, 4, 20],
  [11, 11, 10, 10],
];

const INK = [0x00, 0x00, 0x00, 0xff]; // brand §5 primary mark #000000
const PAPER = [0xff, 0xff, 0xff, 0xff]; // brand §5 inverse mark #FFFFFF
const NAVY = [0x1e, 0x3a, 0x5f, 0xff]; // brand §5 accent #1E3A5F
const CLEAR = [0, 0, 0, 0];

const out = new URL("../public/", import.meta.url);

// --- SVG ------------------------------------------------------------------
const rects = RECTS.map(
  ([x, y, w, h]) => `<rect x="${x}" y="${y}" width="${w}" height="${h}"/>`,
).join("");
const svg =
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">` +
  `<style>g{fill:#000}@media (prefers-color-scheme:dark){g{fill:#fff}}</style>` +
  `<g>${rects}</g></svg>\n`;
writeFileSync(new URL("favicon.svg", out), svg);

// --- PNG ------------------------------------------------------------------
// size: canvas px; scale: px per mark unit; origin: px offset of unit 0.
function raster({ size, scale, origin, fg, bg }) {
  const px = new Uint8Array(size * size * 4);
  for (let i = 0; i < size * size; i++) px.set(bg, i * 4);
  for (const [x, y, w, h] of RECTS) {
    for (let py = origin + y * scale; py < origin + (y + h) * scale; py++) {
      for (let pxx = origin + x * scale; pxx < origin + (x + w) * scale; pxx++) {
        px.set(fg, (py * size + pxx) * 4);
      }
    }
  }
  return png(size, px);
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
function png(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const rows = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    rows[y * (size * 4 + 1)] = 0; // filter: none
    rows.set(rgba.subarray(y * size * 4, (y + 1) * size * 4), y * (size * 4 + 1) + 1);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(rows, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// 32×32 at 1 px/unit: the master canvas, transparent background.
writeFileSync(
  new URL("favicon-32.png", out),
  raster({ size: 32, scale: 1, origin: 0, fg: INK, bg: CLEAR }),
);

// 180×180 apple-touch-icon: a full-bleed plate (iOS rounds the corners
// itself). 4 px/unit makes the visible 28-unit mark 112 px (~62% of the
// plate, brand §9 "~60%"), centered: its extent is units 2..30, so
// unit 0 sits at 90 - 16×4 = 26 px.
writeFileSync(
  new URL("apple-touch-icon.png", out),
  raster({ size: 180, scale: 4, origin: 26, fg: PAPER, bg: NAVY }),
);

console.log("wrote favicon.svg, favicon-32.png, apple-touch-icon.png");
