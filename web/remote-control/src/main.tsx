import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { registerSW } from "virtual:pwa-register";

import { App } from "./app/App";
import "./styles.css";

registerSW({
  immediate: true,
  onRegisterError(error) {
    console.warn("Clinch Remote Control could not register its offline shell", error);
  },
});

const root = document.getElementById("root");
if (!root) throw new Error("Missing Clinch Remote Control root");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
