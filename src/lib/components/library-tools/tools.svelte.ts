// One list's library tools (D34): what is found, how it's sorted, what is
// selected. A page makes one for its rows; the toolbar, the selection bar
// and the table read it. Selection is by entry id, never by position, so
// sorting or loading more doesn't change what is selected.
import { SvelteSet } from "svelte/reactivity";
import { call, type Entry } from "$lib/backend";
import { albumYears, lookUpYears, stopYears } from "$lib/state.svelte";
import { duplicates, find, known, noFacts, sortEntries, startsDescending, type Facts, type SortKey } from "./logic";

export const sortLabels: Record<SortKey, string> = {
  order: "Playlist order",
  title: "Title",
  artist: "Artist",
  album: "Album",
  duration: "Duration",
  year: "Release year",
  plays: "Your plays",
  lastfm: "Last.fm plays",
  youtube: "YouTube plays",
  bpm: "BPM",
};

export interface ListKind {
  /** "playlist", "album", "liked music": for "Find in …". */
  noun: string;
  /** Rows are one album's: no album or year sort. */
  album: boolean;
}

export class ListTools {
  query = $state("");
  key = $state<SortKey>("order");
  desc = $state(false);
  selecting = $state(false);
  /** Reviewing duplicates: only rows in a group show. */
  reviewing = $state(false);
  readonly selected = new SvelteSet<string>();
  facts = $state<Facts>(noFacts());
  #entries: () => Entry[] = () => [];
  #anchor: string | null = null;
  #factsTimer = 0;
  #factsFor = -1;
  readonly kind: ListKind;

  constructor(entries: () => Entry[], kind: ListKind) {
    this.#entries = entries;
    this.kind = kind;
  }

  dups = $derived(duplicates(this.#entries()));
  /** Copies and versions together, for the Duplicates button. */
  dupCount = $derived(this.dups.copies.size + this.dups.versions.size);

  /** The rows as shown: found, sorted, or the duplicate groups. */
  view = $derived.by(() => {
    if (this.reviewing) return this.dups.rows;
    const found = find(this.#entries(), this.query);
    return sortEntries(found, this.key, this.desc, { ...this.facts, years: albumYears });
  });

  /** Sorted, reversed or filtered: the playlist's own order isn't on screen. */
  changed = $derived(this.reviewing || this.query.trim() !== "" || this.key !== "order" || this.desc);
  filtered = $derived(this.reviewing || this.query.trim() !== "");

  /** The sorts this list can offer: a sort with no values isn't one. */
  sorts = $derived.by(() => {
    const es = this.#entries();
    const f = { ...this.facts, years: albumYears };
    const ks: SortKey[] = ["order", "title", "artist"];
    if (!this.kind.album) ks.push("album");
    ks.push("duration");
    if (!this.kind.album && es.some((e) => e.track.album?.id)) ks.push("year");
    if (known(es, "plays", f)) ks.push("plays");
    if (this.facts.lastfm && known(es, "lastfm", f)) ks.push("lastfm");
    // YouTube shows counts on some rows only: offered when most have one.
    if (known(es, "youtube", f) * 2 >= es.length) ks.push("youtube");
    if (known(es, "bpm", f)) ks.push("bpm");
    return ks;
  });

  sortBy(k: SortKey) {
    if (k === this.key) this.desc = !this.desc;
    else {
      this.key = k;
      this.desc = startsDescending(k);
    }
    if (k === "year") void lookUpYears(this.#entries().flatMap((e) => (e.track.album?.id ? [e.track.album.id] : [])));
    else stopYears();
  }

  reverse() {
    this.desc = !this.desc;
  }

  /** Back to the list as the service orders it. */
  reset() {
    this.query = "";
    this.key = "order";
    this.desc = false;
    stopYears();
  }

  /** Plays, Last.fm counts and tempos are local reads: fetched again,
   *  shortly after the rows change. */
  refreshFacts() {
    const es = this.#entries();
    if (es.length === this.#factsFor) return;
    clearTimeout(this.#factsTimer);
    this.#factsTimer = window.setTimeout(async () => {
      this.#factsFor = es.length;
      const rows = es.map((e) => ({ id: e.track.id, counterpart: e.track.counterpart, title: e.track.title, artist: e.track.artists[0]?.name ?? "" }));
      const f = await call((a) => a.features.libraryTools.sortFacts(rows)).catch(() => null);
      if (!f) return;
      const bpm: Record<string, number> = {};
      for (const [id, b] of Object.entries(f.bpm)) if (b != null) bpm[id] = b;
      this.facts = { years: albumYears, plays: f.plays as Record<string, number>, lastfm: (f.lastfm as Record<string, number> | null) ?? null, bpm };
    }, 400);
  }

  // ---------- selection ----------

  start() {
    this.selecting = true;
    this.selected.clear();
    this.#anchor = null;
  }

  /** Review duplicates: copies are selected, versions are for you to check. */
  review() {
    this.selecting = true;
    this.reviewing = true;
    this.selectCopies();
  }

  /** Just the extra copies: every song keeps its first row. */
  selectCopies() {
    this.selected.clear();
    for (const id of this.dups.copies) this.selected.add(id);
  }

  /** In review: the selection is exactly the extra copies. */
  onlyCopies = $derived(this.selected.size === this.dups.copies.size && [...this.dups.copies].every((id) => this.selected.has(id)));

  /** In review, what is selected besides extra copies: first rows, and
   *  the other version (song or video) of a song. */
  beyondCopies = $derived.by(() => {
    let firsts = 0;
    let versions = 0;
    for (const id of this.selected) {
      if (this.dups.copies.has(id)) continue;
      if (this.dups.versions.has(id)) versions++;
      else firsts++;
    }
    return { firsts, versions };
  });

  done() {
    this.selecting = false;
    this.reviewing = false;
    this.selected.clear();
    this.#anchor = null;
  }

  /** Click: one row; Shift-click: every row from the last one clicked. */
  toggle(e: Entry, range: boolean) {
    const view = this.view;
    if (range && this.#anchor) {
      const a = view.findIndex((x) => x.entryId === this.#anchor);
      const b = view.findIndex((x) => x.entryId === e.entryId);
      if (a >= 0 && b >= 0) {
        const on = this.selected.has(this.#anchor);
        for (const x of view.slice(Math.min(a, b), Math.max(a, b) + 1)) {
          if (on) this.selected.add(x.entryId);
          else this.selected.delete(x.entryId);
        }
        return;
      }
    }
    if (this.selected.has(e.entryId)) this.selected.delete(e.entryId);
    else this.selected.add(e.entryId);
    this.#anchor = e.entryId;
  }

  /** Everything shown, or nothing when everything shown already is. */
  toggleAll() {
    const view = this.view;
    if (view.every((e) => this.selected.has(e.entryId))) for (const e of view) this.selected.delete(e.entryId);
    else for (const e of view) this.selected.add(e.entryId);
  }

  allShown = $derived(this.view.length > 0 && this.view.every((e) => this.selected.has(e.entryId)));

  /** The selected rows: those shown in the order shown, then any a search hides. */
  chosen = $derived.by(() => {
    const shown = this.view.filter((e) => this.selected.has(e.entryId));
    const seen = new Set(shown.map((e) => e.entryId));
    return [...shown, ...this.#entries().filter((e) => this.selected.has(e.entryId) && !seen.has(e.entryId))];
  });
}
