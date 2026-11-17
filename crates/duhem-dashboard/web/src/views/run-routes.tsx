// The run report's routes (#694). `/run/:runId` is a layout route: it
// renders `RunScaffold` once and each child renders only its pane through
// the scaffold's `<Outlet>`, so the run fetch, live stream, header and
// rail state survive tab and rail navigation. The diff view has its own
// frame and stays a sibling route. Shared by App and the view tests.

import { Route } from "react-router-dom";

import CheckPage from "./CheckPage";
import CriterionPage from "./CriterionPage";
import DefinitionPage from "./DefinitionPage";
import DiffPage from "./DiffPage";
import LifecyclePage from "./LifecyclePage";
import ResultsPage from "./ResultsPage";
import RunPage from "./RunPage";
import { RunLayout } from "./RunScaffold";

export const runRoutes = (
  <>
    <Route path="/run/:runId" element={<RunLayout />}>
      <Route index element={<RunPage />} />
      <Route path="results" element={<ResultsPage />} />
      <Route path="criterion/:criterionId" element={<CriterionPage />} />
      <Route path="check/:pair" element={<CheckPage />} />
      <Route path="lifecycle/:blockKey" element={<LifecyclePage />} />
      <Route path="definition" element={<DefinitionPage />} />
    </Route>
    <Route path="/run/:runId/diff" element={<DiffPage />} />
  </>
);
