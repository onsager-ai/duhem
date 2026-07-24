// Hash-routed SPA shell. Hash routing (not history-API) is a
// deliberate deviation recorded on the PR for #86/#87: a static
// export must deep-link from any base path on a dumb file host, and
// `#/run/...` routes make that true without per-route HTML copies or
// 404 tricks. The serve-mode fallback to index.html still exists for
// stray non-hash paths.
//
// #284: wrapped in the design-system app shell (sidebar + top bar +
// ⌘K), with Overview at `/`, the runs table at `/runs`, and a
// verifications index at `/verifications`. The evidence views
// (run/check/diff/verification) render inside the shell.
//
// The run report uses summary/results/definition tabs. Results is a
// tree master–detail workspace (criteria → checks in a rail, selected
// evidence in the detail pane). RunPage, ResultsPage, and CheckPage
// share the `RunScaffold` spine so the header persists while drilling.

import { HashRouter, Route, Routes } from "react-router-dom";

import { AppShell } from "@/components/layout/AppShell";
import { Toaster } from "@/components/ui/sonner";
import { RunsProvider } from "@/runs-context";
import { ThemeProvider } from "@/theme";
import CheckPage from "./views/CheckPage";
import CriterionPage from "./views/CriterionPage";
import DefinitionPage from "./views/DefinitionPage";
import DiffPage from "./views/DiffPage";
import Overview from "./views/Overview";
import RunPage from "./views/RunPage";
import ResultsPage from "./views/ResultsPage";
import RunsList from "./views/RunsList";
import VerificationPage from "./views/VerificationPage";
import VerificationsList from "./views/VerificationsList";

export default function App() {
  return (
    <ThemeProvider>
      <HashRouter>
        <RunsProvider>
          <AppShell>
            <Routes>
              <Route path="/" element={<Overview />} />
              <Route path="/runs" element={<RunsList />} />
              <Route path="/verifications" element={<VerificationsList />} />
              <Route path="/run/:runId" element={<RunPage />} />
              <Route path="/run/:runId/results" element={<ResultsPage />} />
              <Route path="/run/:runId/criterion/:criterionId" element={<CriterionPage />} />
              <Route path="/run/:runId/check/:pair" element={<CheckPage />} />
              <Route path="/run/:runId/definition" element={<DefinitionPage />} />
              <Route path="/run/:runId/diff" element={<DiffPage />} />
              <Route path="/verification/:name" element={<VerificationPage />} />
            </Routes>
          </AppShell>
          <Toaster />
        </RunsProvider>
      </HashRouter>
    </ThemeProvider>
  );
}
