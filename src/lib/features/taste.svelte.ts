// Taste (lane D): the block list (D36), "Hide podcasts" (D27), private sessions (D36).
//
// This module is the area's UI state and actions (`call((a) => a.features.taste.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
//
// Rust owns the list and filters pages and the queue with it; this copy
// lets views that already hold a page hide a song the moment it's blocked.
import { call, type BackendError, type Blocked, type BlockKind, type Shelf, type TasteState, type Track } from "$lib/backend";
import { catalog } from "$lib/catalog.svelte";
import { artistLine } from "$lib/data";
import { player, ui } from "$lib/state.svelte";
import { artistMenuItems, deckChips, deckMenuItems, paletteItems, registerFeature, trackMenuItems, type MenuItem } from "./hooks";

/** How long "Undo" stays up (Rust waits a little longer before telling YouTube). */
const UNDO_MS = 8000;

class Taste {
  blocked = $state<Blocked[]>([]);
  hidePodcasts = $state(false);
  /** The offer to undo the last block, shown above the player bar. */
  undo = $state<{ id: number; text: string; kind: BlockKind; target: string } | null>(null);
  /** A clock for the private session's countdown. */
  now = $state(Date.now());

  #tracks = $derived(new Set(this.blocked.filter((b) => b.kind === "track").map((b) => b.target)));
  #artists = $derived(new Set(this.blocked.filter((b) => b.kind === "artist").map((b) => b.target)));

  apply(s: TasteState) {
    this.blocked = s.blocked;
    this.hidePodcasts = s.hidePodcasts;
  }

  /** A song blocked by itself, its other version, or one of its artists. */
  blocks(t: Pick<Track, "id" | "artists" | "counterpart">) {
    return this.#tracks.has(t.id) || (!!t.counterpart && this.#tracks.has(t.counterpart)) || t.artists.some((a) => !!a.id && this.#artists.has(a.id));
  }
  has(kind: BlockKind, id: string) {
    return kind === "track" ? this.#tracks.has(id) : this.#artists.has(id);
  }
  get count() {
    return this.blocked.length;
  }

  /** Shelves as Rust filters them (D36, D27): for pages already on screen. */
  filterShelves(shelves: Shelf[]): Shelf[] {
    if (!this.blocked.length && !this.hidePodcasts) return shelves;
    return shelves
      .map((s) => ({
        ...s,
        items: s.items.filter((i) => {
          if (i.type === "track") return !(this.hidePodcasts && i.kind === "episode") && !this.blocks(i);
          if (i.type === "artist") return !this.#artists.has(i.id);
          if (i.type === "collection") return !(i.artists.length === 1 && !!i.artists[0].id && this.#artists.has(i.artists[0].id) && ["album", "single", "ep"].includes(i.kind));
          return true;
        }),
      }))
      .filter((s) => s.items.length);
  }

  async block(kind: BlockKind, target: string, label: string, detail: string | null) {
    try {
      this.apply(await call((a) => a.features.taste.block(kind, target, label, detail)));
      ui.toast = null;
      const text = kind === "track" ? `“${label}” won't play again` : `${label} won't play again`;
      const id = Date.now();
      this.undo = { id, text, kind, target };
      setTimeout(() => this.undo?.id === id && (this.undo = null), UNDO_MS);
    } catch (e) {
      ui.say(`Could not do that: ${(e as BackendError).message}`, "error");
    }
  }

  async unblock(kind: BlockKind, target: string) {
    if (this.undo?.target === target) this.undo = null;
    try {
      this.apply(await call((a) => a.features.taste.unblock(kind, target)));
    } catch (e) {
      ui.say(`Could not do that: ${(e as BackendError).message}`, "error");
    }
  }

  blockTrack(t: Track) {
    return this.block("track", t.id, t.title, artistLine(t));
  }

  async setHidePodcasts(on: boolean) {
    this.hidePodcasts = on;
    try {
      this.apply(await call((a) => a.features.taste.setHidePodcasts(on)));
    } catch (e) {
      this.hidePodcasts = !on;
      ui.say(`Could not change that: ${(e as BackendError).message}`, "error");
    }
  }

  // ---------- private session ----------

  get private() {
    return !!player.privateSession;
  }
  /** Unix ms the private session ends at; the player keeps the clock (D36). */
  get privateUntil() {
    const r = player.privateSession?.remaining;
    return r == null ? null : player.privateSessionAt + r * 1000;
  }
  /** Whole minutes left, or null when it runs until Encore quits. */
  get privateLeft() {
    return this.privateUntil ? Math.max(1, Math.ceil((this.privateUntil - this.now) / 60000)) : null;
  }

  /** "1 h left", "12 min left", or "" when it runs until Encore quits. */
  get privateLeftText() {
    const m = this.privateLeft;
    return m == null ? "" : m >= 60 ? "1 h left" : `${m} min left`;
  }

  async setPrivate(on: boolean) {
    try {
      await call((a) => a.features.taste.setPrivate(on));
      this.now = Date.now();
      ui.say(on ? "Private session for the next hour. YouTube still sees what you play and search, but it isn't added to your history." : "Private session ended");
    } catch (e) {
      ui.say(`Could not change that: ${(e as BackendError).message}`, "error");
    }
  }

  endPrivate() {
    ui.confirm = {
      title: "End the private session?",
      text: "Songs you play are added to your YouTube history again.",
      confirm: "End session",
      run: () => void this.setPrivate(false),
    };
  }
}

export const taste = new Taste();

const leftText = () => taste.privateLeftText;

registerFeature({
  name: "taste",
  subscribe: () => call((a) => a.features.taste.subscribe((s) => taste.apply(s))),
  load: async () => {
    taste.apply(await call((a) => a.features.taste.state()));
    setInterval(() => (taste.now = Date.now()), 15000);
  },
});

// Track menus: after Download.
trackMenuItems.add(({ track }) => {
  if (taste.has("track", track.id)) return [{ label: "Play this again", icon: "check", run: () => void taste.unblock("track", track.id) }];
  return [{ label: "Don't play this again", icon: "x", run: () => void taste.blockTrack(track) }];
}, 60);

// The artist page's ⋯ menu.
artistMenuItems.add((id) => {
  const name = catalog.artist(id)?.name ?? "This artist";
  if (taste.has("artist", id)) return [{ label: "Play this artist again", icon: "check", run: () => void taste.unblock("artist", id) }];
  return [{ label: "Don't play this artist", icon: "x", run: () => void taste.block("artist", id, name, null) }];
}, 60);

const privateItem = (): MenuItem =>
  taste.private
    ? { label: "End private session", icon: "lock", run: () => void taste.setPrivate(false) }
    : { label: "Start a private session", icon: "lock", run: () => void taste.setPrivate(true) };

deckMenuItems.add(() => [privateItem()], 70);

deckChips.add(() => {
  if (!taste.private) return [];
  const left = leftText();
  const m = taste.privateLeft;
  return [{ key: "private", icon: "lock", label: m == null ? "Private" : `Private ${m >= 60 ? "1h" : `${m}m`}`, title: `Private session${left ? `, ${left}` : ""}: plays aren't added to your YouTube history. Select to end it.`, run: () => taste.endPrivate() }];
}, 60);

// The palette filters these by what's typed.
paletteItems.add(() => {
  const on = taste.private;
  return [{
    key: "taste-private",
    group: "Actions",
    icon: "lock",
    title: on ? "End private session" : "Start a private session",
    sub: on ? `Plays go to your YouTube history again${leftText() ? ` · ${leftText()}` : ""}` : "For an hour, plays aren't added to your YouTube history",
    run: () => void taste.setPrivate(!on),
  }];
});
