// Library tools (lane B): find, sort, select and mass actions, duplicates (D34), the discography and community playlists (D35).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/library_tools.rs). The mock
// is ../mock/libraryTools.ts. Reached as `call((a) => a.features.libraryTools.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.

export interface LibraryToolsApi {}

export const tauriLibraryTools: LibraryToolsApi = {};
