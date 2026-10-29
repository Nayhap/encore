// Reactive cache of everything the backend has sent, keyed by service id,
// plus artwork hues (filled when the art protocol computes them).
import { SvelteMap } from "svelte/reactivity";
import type { AnyArt, ArtRef, Artist, ArtistCard, Collection, CollectionCard, Page, SearchResults, ShelfItem, Track } from "./backend";
import { hashHue } from "./data";

class Catalog {
  tracks = new SvelteMap<string, Track>();
  collections = new SvelteMap<string, CollectionCard | Collection>();
  artists = new SvelteMap<string, ArtistCard | Artist>();
  hues = new SvelteMap<string, [number, number]>();

  track(id: string | null | undefined) {
    return id ? this.tracks.get(id) : undefined;
  }
  collection(id: string | null | undefined) {
    return id ? this.collections.get(id) : undefined;
  }
  artist(id: string | null | undefined) {
    return id ? this.artists.get(id) : undefined;
  }

  addTracks(ts: Track[]) {
    for (const t of ts) {
      const old = this.tracks.get(t.id);
      // Keep a known like state when a later copy does not say.
      this.tracks.set(t.id, old && t.liked == null ? { ...t, liked: old.liked } : t);
      this.addArt(t.art);
    }
  }

  addItems(items: ShelfItem[]) {
    for (const i of items) {
      if (i.type === "track") this.addTracks([i]);
      else if (i.type === "collection") {
        if (!this.collections.has(i.id)) this.collections.set(i.id, i);
        this.addArt(i.art);
      } else if (i.type === "artist") {
        if (!this.artists.has(i.id)) this.artists.set(i.id, i);
        this.addArt(i.art);
      }
    }
  }

  addPage(p: Page) {
    for (const s of p.shelves) this.addItems(s.items);
  }

  addSearch(r: SearchResults) {
    this.addTracks([...r.songs, ...r.videos]);
    this.addItems([...r.albums.map((a) => ({ type: "collection" as const, ...a })), ...r.playlists.map((a) => ({ type: "collection" as const, ...a })), ...r.artists.map((a) => ({ type: "artist" as const, ...a }))]);
  }

  addCollection(c: Collection) {
    this.collections.set(c.id, c);
    this.addTracks(c.entries.map((e) => e.track));
    for (const s of c.related) this.addItems(s.items);
    this.addArt(c.art);
  }

  addArtist(a: Artist) {
    this.artists.set(a.id, a);
    this.addTracks(a.topSongs);
    for (const s of a.shelves) this.addItems(s.items);
    this.addArt(a.art);
  }

  addArt(a: ArtRef | null | undefined) {
    if (a && a.h1 != null && a.h2 != null && !this.hues.has(a.key)) this.hues.set(a.key, [a.h1, a.h2]);
  }

  /** Hues for an artwork: computed ones, else stable ones from the key. */
  hue(a: AnyArt, fallback = ""): [number, number] {
    if (a && "key" in a) {
      const h = this.hues.get(a.key);
      if (h) return h;
      if (a.h1 != null && a.h2 != null) return [a.h1, a.h2];
      const x = hashHue(a.key);
      return [x, (x + 40) % 360];
    }
    if (a && "seed" in a) return [a.h1, a.h2];
    const x = hashHue(fallback || "encore");
    return [x, (x + 40) % 360];
  }
}

export const catalog = new Catalog();
