import React from "react";
import ReactDOM from "react-dom/client";
import { installTauriMock } from "./lib/tauriMock";
import App from "./App";
import "./styles.css";

if (!(window as any).__TAURI_INTERNALS__) {
  installTauriMock();
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
