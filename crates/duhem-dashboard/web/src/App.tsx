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
// (run/check/diff/verification) render inside the shell and are
// reskinned in #285.

import { HashRouter, Route, Routes } from "react-router-dom";

import { AppShell } from "@/components/layout/AppShell";
import { Toaster } from "@/components/ui/sonner";
import { RunsProvider } from "@/runs-context";
import { ThemeProvider } from "@/theme";
import CheckPage from "./views/CheckPage";
import DiffPage from "./views/DiffPage";
import Overview from "./views/Overview";
import RunPage from "./views/RunPage";
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
              {/* Run-scoped report tabs (#280) — each a deep-linkable route
                  rendering the same RunPage shell with a different tab. */}
              <Route path="/run/:runId/suites" element={<RunPage />} />
              <Route path="/run/:runId/categories" element={<RunPage />} />
              <Route path="/run/:runId/timeline" element={<RunPage />} />
              <Route path="/run/:runId/check/:pair" element={<CheckPage />} />
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
