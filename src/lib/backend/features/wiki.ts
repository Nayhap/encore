// Wiki (lane E): Wikipedia on artist pages (D29).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/wiki.rs). The mock
// is ../mock/wiki.ts. Reached as `call((a) => a.features.wiki.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.
import { commands } from "../bindings";
import type { WikiExtract } from "../types";
import { u } from "../unwrap";

export interface WikiApi {
  /** The artist's Wikipedia summary, when one matches strictly. */
  artist(artistId: string, name: string): Promise<WikiExtract | null>;
}

export const tauriWiki: WikiApi = {
  artist: (id, name) => u(commands.wikiArtist(id, name)),
};
