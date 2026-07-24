# Unverified claims

Everything load-bearing I could **not** confirm in this session, and what would confirm it.

| # | Claim (as used) | Why unverified | What confirms it |
|---|---|---|---|
| U1 | **Facts reproducibility can reach ~100% for the geometry subset** under a settling protocol (Gate 1). | No corpus → probe never ran. This is *the* real risk and is entirely open. | Run the Phase-1 Playwright harness N≥5 over ≥20 real pages; report byte-identical-snapshot rate + drift breakdown. |
| U2 | **AI-generated UI defects are meaningfully B/D, not "mostly A"** (Gate 2 fork). | No corpus → failure distribution unmeasured. Determines *build / don't build / build smaller*. | Hardcode the 4 checks (§Phase-2), run over the corpus, report class shares + human-agreement on ~30. |
| U3 | **`inconclusive`/NOI rate is low enough** to be usable (not noise). | Unmeasured; KB flags it as open. | Same Phase-2 run; rank NOI causes. |
| U4 | **The gauntlet actually yields ≥20 distinct real page-states** and they build/serve cleanly for fact collection. | Slate defines 7 apps × evolutions ≈ ~20 states *in principle*; never executed here (no daemon). Tiers B/C blocked on #171. | Run tiers A+D on a docker host with `CHREODE_AGENT=claude`; count served preview pages. |
| U5 | **axe-core is still active/industrial** (brief §2). | I verified Galen (2.4.4, 2019) and DTCG (2025.10 stable) but did **not** search axe-core this session. Not load-bearing for Gate 0. | One release-history check on `dequelabs/axe-core`. |
| U6 | **The economics thesis** (§2.1) — spec cost < value at *our* volume/defect profile. | Depends on U2 (defect profile) + real Chreode page-throughput, neither measured. | Combine U2 distribution with actual pages-per-day from Chreode run telemetry. |
| U7 | **`inconclusive` should become a first-class verdict in `duhem-judge`** (§2 gap, Phase-4 item 4/5). | Architectural + identity-touching (the judge is an identity commitment); code shows judge is currently two-state, but whether tri-state is *warranted* depends on U3. | Human design decision after Gate 2, against KB + `docs/duhem-spec.md` §7.6/§11.2. |
| U8 | **Integration as a Duhem domain pack vs standalone tool** (§Phase-4 item 9). | Provisional recommendation only; brief mandates human confirmation against KB. | Human confirms against KB / spec roadmap §14. |
| U9 | **ReDeCheck's 5 RLF types are the exhaustive L1 relational set.** | KB lists this as its own open question ("是否可穷举"). | Read ReDeCheck source (Phase 3, gated) + measure whether real defects fall outside the 5 types. |
