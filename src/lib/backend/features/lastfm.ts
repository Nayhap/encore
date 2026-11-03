// Last.fm (lane E): connecting, scrobbling and play counts (D31).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/lastfm.rs). The mock
// is ../mock/lastfm.ts. Reached as `call((a) => a.features.lastfm.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface LastfmApi {}

export const tauriLastfm: LastfmApi = {};
