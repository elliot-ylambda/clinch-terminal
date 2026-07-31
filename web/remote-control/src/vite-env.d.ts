/// <reference types="vite/client" />
/// <reference types="vite-plugin-pwa/client" />

declare let self: ServiceWorkerGlobalScope & {
  __WB_MANIFEST: Array<{ url: string; revision: string | null }>;
};
