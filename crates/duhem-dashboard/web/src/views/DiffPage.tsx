// Run-to-run regression diff (#212) with the screenshot visual diff
// (#213). Consumes GET /api/runs/:id/diff (#211): the run vs its
// last-pass baseline. Everything here is evidence for a human to
// read — it renders recorded transitions and pixel/HAR differences,
// and never influences a verdict.

import { useEffect, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import {
  fetchDiff,
  type ArtifactRef,
  type CheckDiff,
  type CriterionDiff,
  type RunDiff,
  type Verdict,
} from "../api";
import { VerdictBadge } from "../ui";
import { changedDelta, diffPixels, harDelta, pickArtifact, type DeltaRow } from "../diff";

function VerdictArrow({ from, to }: { from: Verdict | null; to: Verdict | null }) {
  return (
    <span className="verdict-arrow">
      <VerdictBadge verdict={from} />
      <span className="muted"> → </span>
      <VerdictBadge verdict={to} />
    </span>
  );
}

// #213: side-by-side baseline↔current screenshots with an on-demand
// changed-region overlay. The pixel diff is computed lazily (it's
// heavy) and only when both sides have a same-size screenshot.
function ScreenshotDiff({ base, cur }: { base?: ArtifactRef; cur?: ArtifactRef }) {
  const [overlay, setOverlay] = useState<
    "idle" | "loading" | "error" | "size-mismatch" | { pct: number; url: string }
  >("idle");

  const compute = () => {
    if (!base || !cur) return;
    setOverlay("loading");
    Promise.all([loadImage(base.url), loadImage(cur.url)])
      .then(([a, b]) => {
        const w = a.naturalWidth;
        const h = a.naturalHeight;
        if (w !== b.naturalWidth || h !== b.naturalHeight) {
          setOverlay("size-mismatch");
          return;
        }
        const da = imageData(a, w, h);
        const db = imageData(b, w, h);
        const { pct, mask } = diffPixels(da.data, db.data);
        const out = toCanvas(b, w, h);
        const octx = out.getContext("2d");
        const mc = document.createElement("canvas");
        mc.width = w;
        mc.height = h;
        const mctx = mc.getContext("2d");
        if (mctx) {
          const mimg = mctx.createImageData(w, h);
          mimg.data.set(mask);
          mctx.putImageData(mimg, 0, 0);
        }
        octx?.drawImage(mc, 0, 0);
        setOverlay({ pct, url: out.toDataURL() });
      })
      .catch(() => setOverlay("error"));
  };

  if (!base && !cur) return null;
  return (
    <div className="shot-diff">
      <div className="shot-diff-pair">
        <figure>
          <figcaption className="muted">baseline</figcaption>
          {base ? <img src={base.url} alt="baseline screenshot" /> : <span className="muted">—</span>}
        </figure>
        <figure>
          <figcaption className="muted">current</figcaption>
          {cur ? <img src={cur.url} alt="current screenshot" /> : <span className="muted">—</span>}
        </figure>
      </div>
      {base && cur && (
        <div className="shot-diff-controls">
          {overlay === "idle" && (
            <button type="button" className="linkish" onClick={compute} data-testid="visual-diff-btn">
              show changed regions
            </button>
          )}
          {overlay === "loading" && <span className="muted">computing…</span>}
          {overlay === "error" && <span className="muted">could not compute visual diff</span>}
          {overlay === "size-mismatch" && (
            <span className="muted">screenshots differ in size — no pixel overlay</span>
          )}
          {typeof overlay === "object" && (
            <figure className="shot-diff-overlay">
              <figcaption className="muted">
                changed regions · {(overlay.pct * 100).toFixed(overlay.pct < 0.01 ? 2 : 1)}% of pixels
              </figcaption>
              <img src={overlay.url} alt="visual diff overlay" />
            </figure>
          )}
        </div>
      )}
    </div>
  );
}

function NetworkDelta({ base, cur }: { base?: ArtifactRef; cur?: ArtifactRef }) {
  const [rows, setRows] = useState<DeltaRow[] | "loading" | "error">("loading");
  useEffect(() => {
    if (!base || !cur) {
      setRows([]);
      return;
    }
    let live = true;
    Promise.all([fetch(base.url).then((r) => r.json()), fetch(cur.url).then((r) => r.json())])
      .then(([b, c]) => live && setRows(changedDelta(harDelta(b, c))))
      .catch(() => live && setRows("error"));
    return () => {
      live = false;
    };
  }, [base, cur]);
  if (!base || !cur) return null;
  if (rows === "loading") return <p className="muted">loading network diff…</p>;
  if (rows === "error") return <p className="muted">could not load network diff</p>;
  if (rows.length === 0) return <p className="muted">no network changes</p>;
  return (
    <table className="har net-delta" data-testid="net-delta">
      <thead>
        <tr>
          <th>change</th>
          <th>method</th>
          <th>URL</th>
          <th>baseline → current</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r, i) => (
          <tr key={i} className={r.kind === "status-changed" || r.kind === "new" ? "har-bad" : ""}>
            <td className="net-kind">{r.kind}</td>
            <td>{r.method}</td>
            <td className="har-url" title={r.url}>
              {r.url}
            </td>
            <td>
              {r.baseStatus ?? "—"} → {r.curStatus ?? "—"}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function CheckBlock({ check }: { check: CheckDiff }) {
  const baseShot = pickArtifact(check.baseline_artifacts, "capture/screenshot");
  const curShot = pickArtifact(check.current_artifacts, "capture/screenshot");
  const baseNet = pickArtifact(check.baseline_artifacts, "capture/network");
  const curNet = pickArtifact(check.current_artifacts, "capture/network");
  const flipped = check.assertions.filter((a) => a.changed);
  return (
    <div className={`diff-check ${check.changed ? "changed" : "unchanged"}`}>
      <p className="diff-check-head">
        <strong>{check.id}</strong>{" "}
        {check.changed ? (
          <VerdictArrow from={check.baseline_verdict} to={check.current_verdict} />
        ) : (
          <span className="muted">unchanged</span>
        )}
      </p>
      {flipped.length > 0 && (
        <ul className="diff-assertions">
          {flipped.map((a) => (
            <li key={a.assertion_index}>
              <span className="muted">assertion #{a.assertion_index}: </span>
              <VerdictArrow from={a.baseline_state} to={a.current_state} />
              {(a.baseline_detail || a.current_detail) && (
                <span className="diff-detail">
                  {" — "}
                  {a.baseline_detail && a.baseline_detail !== a.current_detail ? (
                    <>
                      <span className="detail-was">{a.baseline_detail}</span>
                      {" → "}
                      {a.current_detail ?? "(cleared)"}
                    </>
                  ) : (
                    (a.current_detail ?? a.baseline_detail)
                  )}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {check.changed && (baseShot || curShot) && <ScreenshotDiff base={baseShot} cur={curShot} />}
      {check.changed && <NetworkDelta base={baseNet} cur={curNet} />}
    </div>
  );
}

function CriterionBlock({ crit }: { crit: CriterionDiff }) {
  // Changed checks first — the regression is what the reader is here for.
  const checks = [...crit.checks].sort((a, b) => Number(b.changed) - Number(a.changed));
  return (
    <div className="panel diff-criterion">
      <h2>
        {crit.id}{" "}
        {crit.changed ? (
          <VerdictArrow from={crit.baseline_verdict} to={crit.current_verdict} />
        ) : (
          <VerdictBadge verdict={crit.current_verdict} />
        )}
      </h2>
      {checks.map((c) => (
        <CheckBlock key={c.id} check={c} />
      ))}
    </div>
  );
}

export default function DiffPage() {
  const { runId = "" } = useParams();
  const [params] = useSearchParams();
  const baseline = params.get("baseline") ?? undefined;
  const [diff, setDiff] = useState<RunDiff | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDiff(null);
    setError(null);
    fetchDiff(runId, baseline).then(setDiff, (e) => setError(String(e)));
  }, [runId, baseline]);

  const criteria = useMemo(
    () => (diff ? [...diff.criteria].sort((a, b) => Number(b.changed) - Number(a.changed)) : []),
    [diff],
  );

  if (error) return <p className="error">{error}</p>;
  if (diff === null) return <p className="muted">Loading…</p>;

  return (
    <>
      <div className="panel">
        <h2>Regression diff</h2>
        <div className="diff-heads">
          <span>
            <span className="muted">current </span>
            <VerdictBadge verdict={diff.current.verdict} />
          </span>
          <span className="muted"> vs </span>
          {diff.baseline ? (
            <span>
              <span className="muted">baseline </span>
              <VerdictBadge verdict={diff.baseline.verdict} />{" "}
              <Link to={`/run/${encodeURIComponent(diff.baseline.run_id)}`} className="muted">
                {diff.baseline.run_id}
              </Link>
            </span>
          ) : (
            <span className="muted">no baseline</span>
          )}
        </div>
        {diff.baseline === null && (
          <p className="diff-empty" data-testid="diff-no-baseline">
            No prior passing run of this verification to compare against — a diff needs a
            last-known-good baseline. Once this verification passes, later failures will diff
            against that run.
          </p>
        )}
      </div>
      {diff.baseline !== null && criteria.map((c) => <CriterionBlock key={c.id} crit={c} />)}
    </>
  );
}

// ---- canvas helpers (kept here; the pure diff math is in diff.ts) ---

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = url;
  });
}

function toCanvas(img: HTMLImageElement, w: number, h: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  c.getContext("2d")?.drawImage(img, 0, 0);
  return c;
}

function imageData(img: HTMLImageElement, w: number, h: number): ImageData {
  const ctx = toCanvas(img, w, h).getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  return ctx.getImageData(0, 0, w, h);
}
