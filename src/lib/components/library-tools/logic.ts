// Library tools (D34): finding, sorting and duplicates over the rows a page
// has. Pure functions, tested in logic.test.ts (`bun test src/lib`).
import type { Entry, Track } from "$lib/backend";

/** Case, accents and spacing don't matter: "Beyonce" finds "Beyoncé". */
export function fold(s: string): string {
  return s.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase().replace(/\s+/g, " ").trim();
}

// Each track's folded text, made once rather than on every keystroke.
const hays = new WeakMap<Track, string>();
const hay = (t: Track) => {
  let h = hays.get(t);
  if (h == null) hays.set(t, (h = fold([t.title, ...t.artists.map((a) => a.name), t.album?.name ?? ""].join(" "))));
  return h;
};

/** Keys for a keyed list: each id as it is, a repeated one with its
 *  occurrence after it, so no two are the same. */
export function uniqueKeys(ids: string[]): string[] {
  const seen = new Map<string, number>();
  return ids.map((k) => {
    const n = seen.get(k) ?? 0;
    seen.set(k, n + 1);
    return n ? `${k}\u0000${n}` : k;
  });
}

/** Rows whose title, artists or album hold every word of `query`. */
export function find(entries: Entry[], query: string): Entry[] {
  const words = fold(query).split(" ").filter(Boolean);
  if (!words.length) return entries;
  return entries.filter((e) => {
    const h = hay(e.track);
    return words.every((w) => h.includes(w));
  });
}

export type SortKey = "order" | "title" | "artist" | "album" | "duration" | "year" | "plays" | "lastfm" | "youtube" | "bpm";

/** Numbers the UI looks up for some sorts, by track id (or album id for years). */
export interface Facts {
  years: Map<string, number | null>;
  plays: Record<string, number>;
  lastfm: Record<string, number> | null;
  bpm: Record<string, number>;
}

export const noFacts = (): Facts => ({ years: new Map(), plays: {}, lastfm: null, bpm: {} });

/** "48M plays", "1.2B views", "12,345 plays" → a number; `null` if it isn't one. */
export function playCount(s: string | null | undefined): number | null {
  const m = s?.replace(/,/g, "").match(/^\s*([\d.]+)\s*([KMB])?/i);
  if (!m) return null;
  const n = Number(m[1]);
  if (!Number.isFinite(n)) return null;
  const mult = { k: 1e3, m: 1e6, b: 1e9 }[(m[2] ?? "").toLowerCase() as "k" | "m" | "b"] ?? 1;
  return n * mult;
}

/** The value a row sorts by: a string or number, or `null` (sorts last). */
function value(t: Track, key: SortKey, f: Facts): string | number | null {
  switch (key) {
    case "title":
      return t.title;
    case "artist":
      return t.artists[0]?.name ?? null;
    case "album":
      return t.album?.name ?? null;
    case "duration":
      return t.duration ?? null;
    case "year":
      return t.album?.id ? (f.years.get(t.album.id) ?? null) : null;
    case "plays":
      return f.plays[t.id] ?? null;
    case "lastfm":
      return f.lastfm?.[t.id] ?? null;
    case "youtube":
      return playCount(t.plays);
    case "bpm":
      return f.bpm[t.id] ?? null;
    default:
      return null;
  }
}

/** The direction a sort starts in: words A to Z, counts and years highest first. */
export const startsDescending = (k: SortKey) => k === "year" || k === "plays" || k === "lastfm" || k === "youtube";

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/** Sorted copy. Rows without a value go last whichever way; ties keep
 *  the playlist's order, so a sort is stable and undoable. */
export function sortEntries(entries: Entry[], key: SortKey, descending: boolean, f: Facts): Entry[] {
  if (key === "order") return descending ? [...entries].reverse() : entries;
  const rows = entries.map((e, i) => ({ e, i, v: value(e.track, key, f) }));
  rows.sort((a, b) => {
    if (a.v == null || b.v == null) return a.v == null && b.v == null ? a.i - b.i : a.v == null ? 1 : -1;
    const c = typeof a.v === "number" && typeof b.v === "number" ? a.v - b.v : collator.compare(String(a.v), String(b.v));
    return (descending ? -c : c) || a.i - b.i;
  });
  return rows.map((r) => r.e);
}

/** How many rows have a value for `key`: a sort with none isn't offered. */
export const known = (entries: Entry[], key: SortKey, f: Facts) => entries.reduce((n, e) => n + (value(e.track, key, f) == null ? 0 : 1), 0);

// ---------- duplicates ----------

/** A title without the usual version labels, as D21 compares them. */
export function bareTitle(s: string): string {
  return fold(s)
    .replace(/[([](official (music )?video|official audio|official visualizer|lyric video|lyrics|audio|video|mv|hd|4k|remastered( \d{4})?)[)\]]/g, "")
    .replace(/[\s-]+$/, "")
    .trim();
}

export type DupKind = "copy" | "version";

export interface Duplicates {
  /** Rows in a group, in display order: each first row, then its copies. */
  rows: Entry[];
  /** Later copies of the very same row (same video): preselected. */
  copies: Set<string>;
  /** The other version of a song in the list (song ↔ music video): for review. */
  versions: Set<string>;
  /** entryId → what it is, for the row's tag. */
  kind: Map<string, DupKind>;
}

/**
 * Duplicate rows of a playlist. A copy is a row with the same video ID as
 * an earlier row (D10: the row's own ID). A version is a song and its music
 * video: linked by the service (`counterpart`), or a song and a video with
 * the same title and first artist. Versions are never preselected: they
 * are different recordings to YouTube, and may be there on purpose.
 */
export function duplicates(entries: Entry[]): Duplicates {
  const firstByVideo = new Map<string, Entry>();
  const groups = new Map<string, Entry[]>();
  const copies = new Set<string>();
  const versions = new Set<string>();
  const kind = new Map<string, DupKind>();
  // A copy of a row that is itself another's version joins that group.
  const headOf = new Map<string, Entry>();
  const add = (first: Entry, e: Entry, k: DupKind) => {
    const head = headOf.get(first.entryId) ?? first;
    headOf.set(e.entryId, head);
    const g = groups.get(head.entryId) ?? [head];
    g.push(e);
    groups.set(head.entryId, g);
    kind.set(e.entryId, k);
    (k === "copy" ? copies : versions).add(e.entryId);
  };
  const byRecording = new Map<string, Entry>();
  const byName = new Map<string, Entry>();
  const nameKey = (t: Track) => (t.artists[0] ? `${bareTitle(t.title)}\u0000${fold(t.artists[0].name)}` : null);
  for (const e of entries) {
    const first = firstByVideo.get(e.videoId);
    if (first) {
      add(first, e, "copy");
      continue;
    }
    firstByVideo.set(e.videoId, e);
    const ids = [e.videoId, e.track.id, e.track.counterpart].filter((x): x is string => !!x);
    const linked = ids.map((i) => byRecording.get(i)).find((x) => x && x.videoId !== e.videoId);
    const nk = nameKey(e.track);
    const named = nk ? byName.get(nk) : undefined;
    const other = linked ?? (named && named.track.kind !== e.track.kind && (named.track.kind === "video" || e.track.kind === "video") ? named : undefined);
    if (other) add(other, e, "version");
    else {
      for (const i of ids) if (!byRecording.has(i)) byRecording.set(i, e);
      if (nk && !byName.has(nk)) byName.set(nk, e);
    }
  }
  const rows: Entry[] = [];
  for (const e of entries) {
    const g = groups.get(e.entryId);
    if (g) rows.push(...g);
  }
  return { rows, copies, versions, kind };
}
