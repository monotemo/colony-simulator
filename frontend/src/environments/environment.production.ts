/**
 * Production environment (Fly.io).
 *
 * The colony-server on Fly serves this bundle as static files and the
 * simulation runs **in the browser via WebAssembly** — each visitor gets their
 * own per-visitor world (reshuffle / spawn / nectar are local). The pure
 * `colony-core` engine is wrapped by `colony-wasm` and compiled with
 * `wasm-pack` at build time.
 *
 * `backendUrl` is empty because the server is the same origin: flip `useWasm`
 * to `false` to fall back to the live WebSocket transport (one shared
 * server-side world) without changing the URL.
 */
export const environment = {
  useWasm: true,
  backendUrl: '',
};
