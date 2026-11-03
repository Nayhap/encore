// Taste (lane D): the block list (D36), "Hide podcasts" (D27), private sessions (D36).
//
// This module is the area's UI state and actions (`call((a) => a.features.taste.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { registerFeature } from "./hooks";

registerFeature({ name: "taste" });
