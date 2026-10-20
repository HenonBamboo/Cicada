/* Entry module intentionally renders lazy window components and exports no component. */
/* eslint-disable react-refresh/only-export-components */
import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import "../../styles.css";
import { desktopWindowLabel } from "../../lib/desktop";
import { ItemDetails } from "../../components/ItemDetails";

const MainWindow = lazy(() =>
  import("./MainWindow").then((module) => ({ default: module.MainWindow })),
);
const StickyWindow = lazy(() =>
  import("../sticky/StickyWindow").then((module) => ({ default: module.StickyWindow })),
);
const QuickCapture = lazy(() =>
  import("../quick-capture/QuickCapture").then((module) => ({ default: module.QuickCapture })),
);

const label = desktopWindowLabel();
const kind = label === "quick" ? "quick" : label.startsWith("sticky-") ? "sticky" : "main";
document.documentElement.dataset.window = kind;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Suspense fallback={null}>
      {kind === "quick" ? <QuickCapture /> : kind === "sticky" ? <StickyWindow /> : <MainWindow />}
    </Suspense>
    {kind !== "quick" && <ItemDetails />}
  </React.StrictMode>,
);
