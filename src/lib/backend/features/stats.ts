// Stats (lane D): the Stats tab over the local play log (D30).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/stats.rs). The mock
// is ../mock/stats.ts. Reached as `call((a) => a.features.stats.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface StatsApi {}

export const tauriStats: StatsApi = {};
