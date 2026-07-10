/**
 * Development environment (default).
 *
 * Uses the WebSocket transport to talk to a locally running `colony-server`.
 * Replaced by `environment.production.ts` in production builds (see the
 * `fileReplacements` in `angular.json`).
 */
export const environment = {
  /** When true, run the simulation in-browser via WebAssembly. */
  useWasm: false,
  /**
   * Base URL of the colony-server backend (no trailing slash), or empty to talk
   * to the same origin the app is served from. Empty in dev: the Angular dev
   * server proxies `/ws` and `/api` to the local server (see proxy.conf.json).
   */
  backendUrl: '',
  /**
   * When false, skip registering the PWA service worker even in optimized
   * builds. Off only in the desktop (Tauri) build, where the worker buys
   * nothing and isn't emitted; moot in dev, where `isDevMode()` already gates.
   */
  enableServiceWorker: true,
};
