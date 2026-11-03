// The seams the feature modules plug into (see foundation.md in the notes).
// Shared menus, the palette, the deck and the keyboard ask these lists for
// their extra items; each feature module adds its own at load time, so no
// two lanes edit the same menu or command array. This file imports nothing
// from the app's state, so any module can import it without a cycle.

import type { AnyArt, Track } from "$lib/backend";

/** One row of a context menu (`ui.openMenu`). */
export interface MenuItem {
  label: string;
  icon?: string;
  run: () => void;
  danger?: boolean;
  /** Why it can't be used right now (offline, for one); shown instead of running. */
  disabled?: string;
}

/** One row of the ⌘K palette. */
export interface PaletteItem {
  key: string;
  group: string;
  title: string;
  sub: string;
  art?: AnyArt;
  round?: boolean;
  icon?: string;
  hint?: string;
  run: () => void;
  /** Shift+Enter. */
  alt?: () => void;
}

/** A small status chip in the player bar (a sleep timer, a private session). */
export interface DeckChip {
  key: string;
  label: string;
  icon?: string;
  /** Tooltip and accessible name. */
  title: string;
  run?: () => void;
}

/** A single-key shortcut, also listed on the shortcuts sheet (`?`). */
export interface KeyBinding {
  /** `KeyboardEvent.key`, as typed: "a", "b", "M". */
  key: string;
  /** For the sheet: the key as shown, and what it does. */
  label: [string, string];
  run: (e: KeyboardEvent) => void;
}

type Entry<F> = { f: F; order: number; n: number };

/** Contributors to one list. `order` places a contributor among the others
 *  (lower first, 50 by default); equal orders keep load order. */
export class HookList<A extends unknown[], R> {
  #all: Entry<(...a: A) => R[]>[] = [];
  #n = 0;

  add(f: (...a: A) => R[], order = 50): () => void {
    const e = { f, order, n: this.#n++ };
    this.#all = [...this.#all, e].sort((x, y) => x.order - y.order || x.n - y.n);
    return () => (this.#all = this.#all.filter((x) => x !== e));
  }

  /** Every contributor's items, in order. A contributor that throws adds nothing. */
  items(...a: A): R[] {
    return this.#all.flatMap((e) => {
      try {
        return e.f(...a);
      } catch (err) {
        console.error(err);
        return [];
      }
    });
  }
}

/** What a track menu is about: the song, and its queue entry when the menu
 *  was opened on one. */
export interface TrackMenuContext {
  track: Track;
  uid?: string;
}

/** Track menus everywhere: after Download, before "Go to artist". */
export const trackMenuItems = new HookList<[TrackMenuContext], MenuItem>();
/** A playlist's or album's menu (header ⋯, right-click on a card): before "Copy link". */
export const collectionMenuItems = new HookList<[string], MenuItem>();
/** An artist page's ⋯ menu, shown only when something is in it. */
export const artistMenuItems = new HookList<[string], MenuItem>();
/** The player bar's ⋯ menu: after the current song's track menu. */
export const deckMenuItems = new HookList<[], MenuItem>();
/** The player bar's chips, beside the lyrics button. */
export const deckChips = new HookList<[], DeckChip>();
/** Palette commands for the query typed so far (empty: the idle list). */
export const paletteItems = new HookList<[string], PaletteItem>();

const keys = new Map<string, KeyBinding>();

/** A single-key shortcut. Keys the app already uses (Space, S, R, L, D, F,
 *  [, ], /, ?) can't be taken. */
export function addKey(k: KeyBinding) {
  if (keys.has(k.key)) console.error(`key ${k.key} is taken`);
  else keys.set(k.key, k);
}

/** For the app's key handler: true if a feature's shortcut took the key. */
export function runKey(e: KeyboardEvent): boolean {
  const k = keys.get(e.key);
  if (!k) return false;
  k.run(e);
  return true;
}

/** The feature shortcuts, for the shortcuts sheet. */
export const keyLabels = (): [string, string][] => [...keys.values()].map((k) => k.label);

/** A feature module's part in starting up: `subscribe` runs before the
 *  bootstrap (events first, D13), `load` after it. */
export interface FeatureModule {
  name: string;
  subscribe?: () => Promise<unknown>;
  load?: () => Promise<unknown>;
}

const modules: FeatureModule[] = [];

export function registerFeature(m: FeatureModule) {
  modules.push(m);
}

/** Called by `connect()`; a module that fails doesn't stop the others. */
export async function subscribeFeatures() {
  await Promise.all(modules.map((m) => m.subscribe?.().catch((e) => console.error(`${m.name}:`, e))));
}

export async function loadFeatures() {
  await Promise.all(modules.map((m) => m.load?.().catch((e) => console.error(`${m.name}:`, e))));
}
