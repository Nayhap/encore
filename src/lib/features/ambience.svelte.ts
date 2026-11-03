// Ambience (lane E): the cover ambience behind the now playing panel and full-screen lyrics (D27).
//
// This module is the area's UI state and actions (no backend of its own). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
//
// The wash is the cover itself, small and heavily blurred, behind the now
// playing panel and full-screen lyrics only (never behind lists, where it
// would cost legibility). Its setting sits under "Colour from artwork" and
// needs it: off there, off here. Linux keeps D17's plain gradients
// (WebKitGTK blurs on the CPU), so it has no such setting; reduced motion
// drops the crossfade between songs.
import type { AnyArt } from "$lib/backend";
import { player, ui } from "$lib/state.svelte";
import { registerFeature } from "./hooks";

class Ambience {
  /** Settings → Appearance → "Cover ambience" (on unless turned off). */
  enabled = $state(typeof localStorage === "undefined" || localStorage.getItem("ambience") !== "off");

  set(on: boolean) {
    this.enabled = on;
    localStorage.setItem("ambience", on ? "on" : "off");
  }

  /** The wash shows. */
  get on(): boolean {
    return ui.tint && this.enabled && !!player.current;
  }
  get art(): AnyArt {
    return player.current?.art;
  }
  get label(): string {
    return player.current?.title ?? "";
  }
}

export const ambience = new Ambience();

/** Crossfades between covers, unless the system asks for less motion. */
export const washFade = () => (typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 900);

registerFeature({ name: "ambience" });
