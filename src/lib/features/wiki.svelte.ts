// Wiki (lane E): Wikipedia on artist pages (D29).
//
// This module is the area's UI state and actions (`call((a) => a.features.wiki.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { SvelteMap } from "svelte/reactivity";
import { call, type WikiExtract } from "$lib/backend";
import { registerFeature } from "./hooks";

/** Answers this session, by artist id; `null` means none matched. Rust
 *  keeps them for 30 days. */
const known = new SvelteMap<string, WikiExtract | null>();
const asking = new Map<string, Promise<void>>();

/** The artist's Wikipedia summary, once asked for (`undefined` while asking). */
export function wikiFor(artistId: string, name: string): WikiExtract | null | undefined {
  if (!known.has(artistId) && !asking.has(artistId)) {
    asking.set(
      artistId,
      call((a) => a.features.wiki.artist(artistId, name))
        .catch(() => null)
        .then((w) => void known.set(artistId, w))
        .finally(() => asking.delete(artistId)),
    );
  }
  return known.get(artistId);
}

registerFeature({ name: "wiki" });
