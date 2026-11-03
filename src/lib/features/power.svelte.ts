// Power (lane E): the Power Bar: prefixes, seeking, recent items in the palette (D32).
//
// This module is the area's UI state and actions (no backend of its own). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
//
// The palette (components/Palette.svelte) reads what was typed through
// ./powerQuery. Here: what it remembers between openings.
import type { AnyArt, Track } from "$lib/backend";
import { registerFeature, type PaletteItem } from "./hooks";

/** Something opened or played from the palette, shown when it opens empty. */
export interface PowerRecent {
  kind: "track" | "artist" | "collection";
  id: string;
  title: string;
  sub: string;
  art?: AnyArt;
  round?: boolean;
  /** Songs keep their track, to play without asking again. */
  track?: Track;
}

/** Contributed palette groups that list things rather than commands: `>`
 *  leaves them out, `#` takes the folders. */
export const POWER_THINGS = new Set(["Bookmarks", "Saved queues", "Folders", "Downloaded songs"]);

/** A contributed palette item that does something (not a thing to open). */
export const isCommand = (i: PaletteItem) => i.art === undefined && !POWER_THINGS.has(i.group);

const KEY = "palette.recent";
const MAX = 6;

function load(): PowerRecent[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.slice(0, MAX) : [];
  } catch {
    return [];
  }
}

class Power {
  recent = $state<PowerRecent[]>(typeof localStorage === "undefined" ? [] : load());

  remember(r: PowerRecent) {
    this.recent = [r, ...this.recent.filter((x) => !(x.kind === r.kind && x.id === r.id))].slice(0, MAX);
    localStorage.setItem(KEY, JSON.stringify(this.recent));
  }

  forget() {
    this.recent = [];
    localStorage.removeItem(KEY);
  }
}

export const power = new Power();

registerFeature({ name: "power" });
