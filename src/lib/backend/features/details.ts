// Details (lane E): the song details sheet, tempo and key (D28).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/details.rs). The mock
// is ../mock/details.ts. Reached as `call((a) => a.features.details.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface DetailsApi {}

export const tauriDetails: DetailsApi = {};
