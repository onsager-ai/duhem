// THROWAWAY PROBE — merge Phase 1 shards and emit the report table.
import fs from 'node:fs';
const shards = fs.readdirSync('out').filter(f => /^phase1\.shard\d+\.json$/.test(f));
let report = [], driftTally = {}, N = 5;
for (const f of shards) {
  const d = JSON.parse(fs.readFileSync('out/' + f, 'utf8'));
  N = d.N; report = report.concat(d.report);
  for (const [k, v] of Object.entries(d.driftTally)) driftTally[k] = (driftTally[k] || 0) + v;
}
report.sort((a, b) => a.genre.localeCompare(b.genre) || a.id.localeCompare(b.id));
const ok = report.filter(r => r.settled && !r.settled.error);
const okn = report.filter(r => r.naive && !r.naive.error);
const mean = (arr, f) => +(arr.reduce((a, r) => a + f(r), 0) / Math.max(arr.length, 1)).toFixed(2);

console.log('| page | genre | naive % | settled % | geom % | set stable | elements |');
console.log('|---|---|---|---|---|---|---|');
for (const r of report) {
  const s = r.settled, n = r.naive;
  console.log(`| ${r.id} | ${r.genre} | ${n?.error ? 'ERR' : n.identicalPct} | ${s?.error ? 'ERR' : s.identicalPct} | ${s?.error ? '-' : s.geomPct} | ${s?.error ? '-' : (s.setStable ? 'yes' : 'NO')} | ${s?.error ? '-' : s.n} |`);
}
console.log('\npages:', report.length, ' N runs each:', N);
console.log('mean identical naive :', mean(okn, r => r.naive.identicalPct));
console.log('mean identical settled:', mean(ok, r => r.settled.identicalPct));
console.log('mean geom settled     :', mean(ok, r => r.settled.geomPct));
console.log('pages 100% all-fields settled:', ok.filter(r => r.settled.identicalPct === 100).length, '/', ok.length);
console.log('pages 100% geometry  settled:', ok.filter(r => r.settled.geomPct === 100).length, '/', ok.length);
console.log('pages 100% geometry  naive  :', okn.filter(r => r.naive.geomPct === 100).length, '/', okn.length);
console.log('pages with unstable element set:', ok.filter(r => !r.settled.setStable).map(r => r.id).join(', ') || 'none');
const totalEls = ok.reduce((a, r) => a + r.settled.n, 0);
console.log('total elements compared:', totalEls);
// element-weighted rates
const wGeom = ok.reduce((a, r) => a + r.settled.n * r.settled.geomPct / 100, 0);
const wAll = ok.reduce((a, r) => a + r.settled.n * r.settled.identicalPct / 100, 0);
console.log('element-weighted geom identical  :', +(100 * wGeom / totalEls).toFixed(3) + '%');
console.log('element-weighted all-field ident.:', +(100 * wAll / totalEls).toFixed(3) + '%');
console.log('\ndrift fields (settled, element-instances):');
for (const [k, v] of Object.entries(driftTally).sort((a, b) => b[1] - a[1]).slice(0, 15)) console.log('  ', k.padEnd(22), v);
fs.writeFileSync('out/phase1.merged.json', JSON.stringify({ N, report, driftTally }, null, 2));
