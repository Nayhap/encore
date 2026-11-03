// Ambience (lane E): the cover ambience behind the now playing panel and full-screen lyrics (D27).
//
// This module is the area's UI state and actions (no backend of its own). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { registerFeature } from "./hooks";

registerFeature({ name: "ambience" });
