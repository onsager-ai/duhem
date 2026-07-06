// Hash-routed SPA shell. Hash routing (not history-API) is a
// deliberate deviation recorded on the PR for #86/#87: a static
// export must deep-link from any base path on a dumb file host, and
// `#/run/...` routes make that true without per-route HTML copies or
// 404 tricks. The serve-mode fallback to index.html still exists for
// stray non-hash paths.

import { HashRouter, Route, Routes } from "react-router-dom";
import RunsList from "./views/RunsList";
import RunPage from "./views/RunPage";
import CheckPage from "./views/CheckPage";
import VerificationPage from "./views/VerificationPage";

export default function App() {
  return (
    <HashRouter>
      <header className="app">
        <h1>Duhem</h1>
        <span className="sub">runs &amp; evidence — read-only</span>
      </header>
      <main>
        <Routes>
          <Route path="/" element={<RunsList />} />
          <Route path="/run/:runId" element={<RunPage />} />
          <Route path="/run/:runId/check/:pair" element={<CheckPage />} />
          <Route path="/verification/:name" element={<VerificationPage />} />
        </Routes>
      </main>
    </HashRouter>
  );
}
