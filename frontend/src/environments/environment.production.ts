/**
 * Production environment (GitHub Pages).
 *
 * Runs the simulation in-browser via WebAssembly, since Pages is static and
 * cannot host the Rust server.
 */
export const environment = {
  useWasm: true,
};
