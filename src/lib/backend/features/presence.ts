// Presence (lane E): Discord Rich Presence (D31).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/presence.rs). The mock
// is ../mock/presence.ts. Reached as `call((a) => a.features.presence.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface PresenceApi {}

export const tauriPresence: PresenceApi = {};
