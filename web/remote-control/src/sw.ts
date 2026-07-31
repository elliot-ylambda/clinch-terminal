/// <reference lib="webworker" />

import { clientsClaim } from "workbox-core";
import { cleanupOutdatedCaches, createHandlerBoundToURL, precacheAndRoute } from "workbox-precaching";
import { NavigationRoute, registerRoute } from "workbox-routing";

declare let self: ServiceWorkerGlobalScope & { __WB_MANIFEST: Array<unknown> };

self.skipWaiting();
clientsClaim();
cleanupOutdatedCaches();
precacheAndRoute(self.__WB_MANIFEST);

// Only the immutable app shell is available offline. API, WebSocket, terminal, prompt, usage,
// pairing, and credential traffic is never handled or cached by this worker.
registerRoute(
  new NavigationRoute(createHandlerBoundToURL("index.html"), {
    denylist: [/\/api\//, /\/ws$/],
  }),
);
