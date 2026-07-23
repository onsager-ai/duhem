import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
// globals.css: Tailwind + design tokens (owns the theme). styles.css:
// the not-yet-reskinned evidence-view component rules (#285 migrates
// them onto the design system, then this second import goes away).
import "./globals.css";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
