// Renders the real demo output (from ./run.sh) into a looping animated
// terminal SVG for the README. No external deps — plain Node. Every line
// of text below is real output captured from two actual `duhem run`s
// (broken front, then fixed); only the pacing is synthesized.
//
//   node render-svg.mjs > demo.svg
import { writeFileSync } from "node:fs";

const T = 9500;            // loop duration (ms)
const charW = 8.4, lineH = 20, padX = 18, padTop = 46, padBottom = 16, font = 14;

const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const P = ["$ ", "p"];   // prompt segment shorthand [text, class]

// Each line: { at } appear time (ms), and segments [text, class]. null line = blank row.
const L = [
  { at: 600,  segs: [P, ["curl -s localhost:8477/health", "c"]] },
  { at: 1200, segs: [['{"status":"healthy"}', "o"], ["    # the app says it's healthy", "d"]] },
  null,
  { at: 2100, segs: [P, ["duhem run", "c"]] },
  { at: 2800, segs: [["fail", "f"]] },
  { at: 3000, segs: [["  AC-2::AC-2.1:", "o"]] },
  { at: 3200, segs: [['    fail  $steps.front.outputs.body.title == "Acme Dashboard"', "f"]] },
  { at: 3400, segs: [['        (actual "Welcome to nginx!", expected "Acme Dashboard")', "d"]] },
  null,
  { at: 4300, segs: [["# /health was green — Duhem drove the real front and caught it", "m"]] },
  null,
  { at: 5100, segs: [P, ["# ship the fix, re-run the same gate", "d"]] },
  { at: 5700, segs: [P, ["duhem run", "c"]] },
  { at: 6400, segs: [["pass", "g"]] },
];

let maxCol = 0;
for (const ln of L) if (ln) maxCol = Math.max(maxCol, ln.segs.reduce((n, [t]) => n + t.length, 0));

const W = Math.ceil(padX * 2 + maxCol * charW);
const H = padTop + L.length * lineH + padBottom;

const pct = (ms) => Math.max(0, Math.min(100, (ms / T) * 100));

const tspansFor = (ln) => {
  let col = 0;
  return ln.segs
    .map(([t, c]) => {
      const x = padX + col * charW;
      col += t.length;
      return `<tspan x="${x.toFixed(1)}" class="${c}">${esc(t)}</tspan>`;
    })
    .join("");
};

const head = (extra) => `<svg xmlns="http://www.w3.org/2000/svg" xml:space="preserve" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" font-family="ui-monospace,'SF Mono','Cascadia Code','Fira Code',Menlo,Consolas,monospace" font-size="${font}">
<style>
  .line{white-space:pre}
  .p{fill:#56d4bb}.c{fill:#e6edf3}.o{fill:#c9d1d9}.d{fill:#7d8590}.f{fill:#f85149}.g{fill:#3fb950}.m{fill:#d29922}
  ${extra}
</style>
<rect width="${W}" height="${H}" rx="10" fill="#0d1117"/>
<rect width="${W}" height="30" rx="10" fill="#161b22"/><rect y="16" width="${W}" height="14" fill="#161b22"/>
<circle cx="18" cy="15" r="5" fill="#f85149"/><circle cx="36" cy="15" r="5" fill="#d29922"/><circle cx="54" cy="15" r="5" fill="#3fb950"/>
<text x="${W / 2}" y="19" text-anchor="middle" font-size="11" fill="#7d8590">duhem — catching a self-masking bug</text>`;

// `--at <ms>` emits one static frame (each line shown iff revealed by then)
// for GIF assembly; default emits the looping animated demo.svg.
const ai = process.argv.indexOf("--at");
if (ai !== -1) {
  const at = Number(process.argv[ai + 1]);
  let body = "";
  L.forEach((ln, row) => {
    if (!ln) return;
    const y = padTop + row * lineH + font;
    body += `<text y="${y}" class="line" style="opacity:${at >= ln.at ? 1 : 0}">${tspansFor(ln)}</text>`;
  });
  process.stdout.write(`${head("")}\n${body}\n</svg>\n`);
} else {
  let keyframes = "", body = "";
  L.forEach((ln, row) => {
    if (!ln) return;
    const y = padTop + row * lineH + font;
    const p = pct(ln.at);
    keyframes += `@keyframes ln${row}{0%{opacity:0}${Math.max(0, p - 0.5).toFixed(2)}%{opacity:0}${p.toFixed(2)}%{opacity:1}97%{opacity:1}100%{opacity:0}}`;
    body += `<text y="${y}" class="line" xml:space="preserve" style="animation:ln${row} ${T}ms infinite">${tspansFor(ln)}</text>`;
  });
  writeFileSync(new URL("./demo.svg", import.meta.url), `${head(keyframes)}\n${body}\n</svg>\n`);
  process.stderr.write(`wrote demo.svg  ${W}x${H}\n`);
}
