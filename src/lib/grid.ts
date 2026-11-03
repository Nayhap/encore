// "Show all" pages as a grid: what each item is, its year, and the
// find / kind / sort view over them. Pure, so it's tested on its own.
import type { ShelfItem } from "$lib/backend";

export type GridSort = "default" | "newest" | "oldest" | "title";

/** The kind a chip filters by: a collection's kind, else the item's type. */
export function kindOf(it: ShelfItem): string {
  if (it.type === "collection") return it.kind === "liked" ? "playlist" : it.kind;
  if (it.type === "track") return it.kind === "video" ? "video" : "song";
  return it.type;
}

/** A collection's year, or the one in its subtitle ("Album • 2024"). */
export function yearOf(it: ShelfItem): number | null {
  if (it.type === "collection" && it.year) return it.year;
  const sub = it.type === "collection" || it.type === "artist" ? it.subtitle : null;
  const m = sub ? /\b(19|20)\d{2}\b/.exec(sub) : null;
  return m ? Number(m[0]) : null;
}

export function titleOf(it: ShelfItem): string {
  return it.type === "artist" ? it.name : it.title;
}

function subOf(it: ShelfItem): string {
  if (it.type === "collection") return [it.subtitle, ...it.artists.map((a) => a.name)].join(" ");
  if (it.type === "track") return it.artists.map((a) => a.name).join(" ");
  if (it.type === "artist") return it.subtitle ?? "";
  return "";
}

const fold = (s: string) => s.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/** Every word of the query, in the title or the line under it. */
export function matches(it: ShelfItem, query: string): boolean {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = fold(`${titleOf(it)} ${subOf(it)}`);
  return words.every((w) => hay.includes(w));
}

/** Kinds present, in a fixed order, with their counts. */
export function kindsIn(items: ShelfItem[]): { kind: string; count: number }[] {
  const order = ["album", "single", "ep", "playlist", "mix", "song", "video", "artist", "link"];
  const counts = new Map<string, number>();
  for (const it of items) counts.set(kindOf(it), (counts.get(kindOf(it)) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])).map(([kind, count]) => ({ kind, count }));
}

/** The items shown: filtered by kind and query, then sorted. Unknown years
 *  sort last either way; ties keep YouTube's order. */
export function view(items: ShelfItem[], o: { kind: string | null; query: string; sort: GridSort }): ShelfItem[] {
  const out = items.map((it, i) => ({ it, i })).filter(({ it }) => (!o.kind || kindOf(it) === o.kind) && matches(it, o.query));
  if (o.sort === "title") out.sort((a, b) => titleOf(a.it).localeCompare(titleOf(b.it), undefined, { sensitivity: "base", numeric: true }) || a.i - b.i);
  else if (o.sort !== "default") {
    const dir = o.sort === "newest" ? -1 : 1;
    out.sort((a, b) => {
      const ya = yearOf(a.it);
      const yb = yearOf(b.it);
      if (ya == null || yb == null) return ya == null && yb == null ? a.i - b.i : ya == null ? 1 : -1;
      return (ya - yb) * dir || a.i - b.i;
    });
  }
  return out.map((x) => x.it);
}

export const kindNames: Record<string, [string, string]> = {
  album: ["Album", "Albums"],
  single: ["Single", "Singles"],
  ep: ["EP", "EPs"],
  playlist: ["Playlist", "Playlists"],
  mix: ["Mix", "Mixes"],
  song: ["Song", "Songs"],
  video: ["Video", "Videos"],
  artist: ["Artist", "Artists"],
  link: ["Page", "Pages"],
};

/** "12 albums", or "30 items" for a mix of kinds. */
export function countLabel(n: number, kinds: string[]): string {
  const name = kinds.length === 1 ? kindNames[kinds[0]]?.[n === 1 ? 0 : 1].toLowerCase() : n === 1 ? "item" : "items";
  return `${n.toLocaleString()} ${name ?? "items"}`;
}
