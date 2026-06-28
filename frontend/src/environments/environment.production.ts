/**
 * Production environment (GitHub Pages).
 *
 * Pages is static and can't host the Rust server, so the frontend talks to the
 * colony-server deployed on Fly.io over WebSocket. All visitors share that one
 * server-side simulation — a single live world they collectively watch and
 * perturb (reshuffle / spawn / nectar affect everyone). The in-browser wasm
 * engine is still built and shipped, but unused while `useWasm` is false; flip
 * it back to `true` to fall back to a per-visitor in-browser simulation.
 */
export const environment = {
  useWasm: false,
  backendUrl: 'https://colony-simulator-api.fly.dev',
};
