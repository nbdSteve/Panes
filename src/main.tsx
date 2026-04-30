import React from "react";
import ReactDOM from "react-dom/client";
import { installTauriMock } from "./lib/tauriMock";
import App from "./App";
import "./styles.css";

async function init() {
  if (!(window as any).__TAURI_INTERNALS__) {
    if (import.meta.env.VITE_FULLSTACK_TEST) {
      const { installTauriBridge } = await import("./lib/tauriBridge");
      await installTauriBridge();
    } else {
      installTauriMock();
    }
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

init().catch((e) => {
  document.body.textContent = `Init failed: ${e.message}`;
});
