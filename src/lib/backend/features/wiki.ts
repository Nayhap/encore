// Wiki (lane E): Wikipedia extracts on artist pages (D29).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/wiki.rs). The mock
// is ../mock/wiki.ts. Reached as `call((a) => a.features.wiki.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface WikiApi {}

export const tauriWiki: WikiApi = {};
