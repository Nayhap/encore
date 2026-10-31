// Display helpers shared by the views. Catalogue data comes from the backend
// (`$lib/backend`), cached in `$lib/catalog.svelte`.
import type { CollectionKind, LyricLine, Track } from "./backend";

export type { Track, LyricLine } from "./backend";

export const fmt = (s: number | null | undefined) => {
  if (s == null || !Number.isFinite(s)) return "–:––";
  s = Math.max(0, Math.floor(s));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
};

export const totalDuration = (tracks: Track[]) => {
  const s = tracks.reduce((n, t) => n + (t.duration ?? 0), 0);
  const h = Math.floor(s / 3600);
  const m = Math.round((s % 3600) / 60);
  return h ? `${h} hr ${m} min` : `${m} min`;
};

export const kindLabel: Record<CollectionKind, string> = {
  album: "Album",
  single: "Single",
  ep: "EP",
  playlist: "Playlist",
  mix: "Mix",
  liked: "Playlist",
};

/** Bytes for people: "340 MB", "1.2 GB". */
export const sizeText = (b: number | null | undefined) => {
  const n = b ?? 0;
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  return n > 0 ? "less than 1 MB" : "0 MB";
};

export const artistLine = (t: Pick<Track, "artists">) => t.artists.map((a) => a.name).join(", ") || "Unknown artist";

/** Stable hue from a string, used until a cover's real hue is known. */
export function hashHue(s: string) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return ((h >>> 0) % 360);
}

/** Song structure from synced lyrics: a line repeated 3+ times is chorus, twice is a hook. */
export function lyricSegments(lines: LyricLine[] | null | undefined, dur: number | null | undefined) {
  if (!lines?.length || !dur) return null;
  const counts = new Map<string, number>();
  for (const l of lines) counts.set(l.text, (counts.get(l.text) ?? 0) + 1);
  return lines.map((l, i) => {
    const t = l.t ?? 0;
    const next = lines[i + 1]?.t ?? dur;
    const est = Math.min(Math.max(l.text.length * 0.11 + 1.6, 2.4), 4.6);
    const end = l.end != null ? Math.min(l.end, next - 0.35) : Math.min(next - 0.35, t + est);
    const c = counts.get(l.text)!;
    return {
      start: t,
      end: Math.max(end, t + 0.5),
      next,
      text: l.text,
      tier: (c >= 3 ? "chorus" : c === 2 ? "hook" : "verse") as "chorus" | "hook" | "verse",
    };
  });
}

export type Segment = NonNullable<ReturnType<typeof lyricSegments>>[number];
