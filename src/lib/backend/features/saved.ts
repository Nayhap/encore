// Saved (lane C): bookmarks (D25), saved queues (D33), folders and their covers (D34).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/saved.rs). The mock
// is ../mock/saved.ts. Reached as `call((a) => a.features.saved.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface SavedApi {}

export const tauriSaved: SavedApi = {};
