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
};
