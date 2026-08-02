import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { registerSW } from "virtual:pwa-register";

import "@fontsource-variable/inter/wght.css";
import "@fontsource-variable/jetbrains-mono/wght.css";

import { App } from "./app/App";
import "./styles.css";

// The previous app shell can remain visible after an updated service worker activates. Reload the
// already-controlled page once so links opened from Clinch Settings always settle on the current
// hashed assets without asking the user for a second manual refresh.
const serviceWorker = "serviceWorker" in navigator ? navigator.serviceWorker : undefined;
if (serviceWorker?.controller) {
  let reloadingForUpdate = false;
  serviceWorker.addEventListener("controllerchange", () => {
    if (reloadingForUpdate) return;
    reloadingForUpdate = true;
    window.location.reload();
  });
}

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
