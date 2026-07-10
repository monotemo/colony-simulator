/**
 * Desktop environment (Tauri shell).
 *
 * Identical to production — the simulation runs in-process via WebAssembly, no
 * server — except the service worker is disabled: Tauri serves the bundle over
 * its own protocol where a PWA service worker buys nothing (the app is already
 * installed and offline) and registration is unreliable. The `desktop` build
 * configuration in `angular.json` pairs this with dropping the `serviceWorker`
 * option so `ngsw-worker.js` is never emitted.
 */
export const environment = {
  useWasm: true,
  backendUrl: '',
  enableServiceWorker: false,
};
