// Playback (lane A): the A–B loop and the sleep timer (D26), shuffle cycles and the Autoplay rule (D33).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/playback.rs). The mock
// is ../mock/playback.ts. Reached as `call((a) => a.features.playback.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface PlaybackApi {}

export const tauriPlayback: PlaybackApi = {};
