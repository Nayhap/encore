// Taste (lane D): the block list (D36), "Hide podcasts" (D27), private sessions (D36).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/taste.rs). The mock
// is ../mock/taste.ts. Reached as `call((a) => a.features.taste.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface TasteApi {}

export const tauriTaste: TasteApi = {};
