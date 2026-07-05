/**
 * Production environment (Cloudflare Workers, static assets only).
 *
 * The deployed origin serves nothing but this bundle and the simulation runs
 * **in the browser via WebAssembly** — each visitor gets their own per-visitor
 * world (reshuffle / spawn / nectar are local). The pure `colony-core` engine
 * is wrapped by `colony-wasm` and compiled with `wasm-pack` at build time.
 *
 * There is no server behind the deployed origin, so flipping `useWasm` to
 * `false` (the live WebSocket transport, one shared server-side world) also
 * means pointing `backendUrl` at a colony-server you host — empty means
 * "same origin", which only works where colony-server itself serves the
 * bundle (local dev, or the self-hosted Docker image).
 */
export const environment = {
  useWasm: true,
  backendUrl: '',
};
