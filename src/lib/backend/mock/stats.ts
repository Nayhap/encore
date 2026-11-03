// The mock of stats (lane D) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
//
// A made-up play log of the last five months (the same every time), plus
// whatever the simulated player plays. `?nostats` starts with none, as on a
// fresh install. The sums follow encore-core's `Store::stats`.
import * as M from "../mockdata";
import { mockKit } from "../mock";
import type { StatsApi } from "../features/stats";
import type * as T from "../types";

interface Play {
  id: string;
  /** Local time, ms. */
  at: number;
  heard: number;
  counted: boolean;
}

const q = new URLSearchParams(location.search);
const log: Play[] = [];
const subs = new Set<() => void>();

// A small deterministic random (mulberry32).
function rng(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

if (!q.has("nostats")) {
  const r = rng(7);
  const ids = M.tracks.map((t) => t.id);
  const now = Date.now();
  const start = new Date(new Date().getFullYear(), new Date().getMonth() - 4, 1).getTime();
  // Mornings and late evenings, mostly.
  const hours = [7, 8, 8, 9, 12, 13, 17, 18, 19, 20, 21, 21, 22, 22, 23, 0];
  for (let day = start; day < now; day += 86400000) {
    const n = Math.floor(r() * 9);
    // Each month leans on a different handful of songs.
    const m = new Date(day).getMonth();
    for (let i = 0; i < n; i++) {
      const pick = r() < 0.6 ? ids[(m * 5 + Math.floor(r() * 7)) % ids.length] : ids[Math.floor(r() * ids.length)];
      const t = M.tracks.find((x) => x.id === pick)!;
      const at = day + hours[Math.floor(r() * hours.length)] * 3600000 + Math.floor(r() * 3600000);
      if (at > now) continue;
      const skip = r() < 0.15;
      const heard = skip ? 5 + r() * 30 : t.dur * (0.8 + r() * 0.2);
      log.push({ id: `mock:${t.id}`, at, heard, counted: !skip });
    }
  }
}

/** The simulated player played `t` (it counts at once, for the demo). */
export function mockStatsPlayed(t: T.Track) {
  log.push({ id: t.id, at: Date.now(), heard: (t.duration ?? 180) * 0.9, counted: true });
  subs.forEach((h) => h());
}

const inPeriod = (p: T.StatsPeriod) => (x: Play) => {
  const d = new Date(x.at);
  return d.getFullYear() === p.year && (p.month == null || d.getMonth() + 1 === p.month);
};

function rank<K>(m: Map<K, { plays: number; heard: number }>) {
  return [...m].filter(([, t]) => t.plays > 0).sort((a, b) => b[1].plays - a[1].plays || b[1].heard - a[1].heard);
}

function stats(p: T.StatsPeriod): T.ListeningStats {
  const rows = log.filter(inPeriod(p));
  const bySong = new Map<string, { plays: number; heard: number }>();
  const byHour = Array(24).fill(0);
  for (const r of rows) {
    const s = bySong.get(r.id) ?? { plays: 0, heard: 0 };
    s.plays += +r.counted;
    s.heard += r.heard;
    bySong.set(r.id, s);
    byHour[new Date(r.at).getHours()] += r.heard / 60;
  }
  const songs = rank(bySong);
  const credit = (key: (t: T.Track) => T.Credit | null | undefined, album: boolean): T.TopCredit[] => {
    const m = new Map<string, { plays: number; heard: number; t: T.Track }>();
    for (const [id, s] of songs) {
      const t = mockKit.track(id);
      const c = key(t);
      if (!c) continue;
      const e = m.get(c.id ?? c.name) ?? { plays: 0, heard: 0, t };
      e.plays += s.plays;
      e.heard += s.heard;
      m.set(c.id ?? c.name, e);
    }
    return [...m.values()]
      .sort((a, b) => b.plays - a.plays || b.heard - a.heard)
      .slice(0, 10)
      .map((e) => ({ ...key(e.t)!, art: e.t.art, detail: album ? e.t.artists.map((a) => a.name).join(", ") : null, plays: e.plays, minutes: e.heard / 60 }));
  };
  const first = new Map<string, number>();
  for (const r of log) if (r.counted && (first.get(r.id) ?? Infinity) > r.at) first.set(r.id, r.at);
  const isNew = (id: string) => inPeriod(p)({ id, at: first.get(id) ?? 0, heard: 0, counted: true });
  const fresh = songs.filter(([id]) => isNew(id));
  return {
    period: p,
    minutes: rows.reduce((s, r) => s + r.heard, 0) / 60,
    plays: rows.filter((r) => r.counted).length,
    songs: songs.length,
    topSongs: songs.slice(0, 10).map(([id, s]) => ({ track: mockKit.track(id), plays: s.plays, minutes: s.heard / 60 })),
    topArtists: credit((t) => t.artists[0], false),
    topAlbums: credit((t) => t.album, true),
    byHour,
    newSongs: fresh.length,
    newTop: fresh.slice(0, 8).map(([id]) => mockKit.track(id)),
  };
}

export const mockStats: StatsApi = {
  overview() {
    const months = new Map<string, T.StatsPeriod>();
    for (const r of [...log].sort((a, b) => b.at - a.at)) {
      const d = new Date(r.at);
      months.set(`${d.getFullYear()}-${d.getMonth()}`, { year: d.getFullYear(), month: d.getMonth() + 1 });
    }
    const counted = log.filter((r) => r.counted);
    const since = counted.length ? Math.min(...counted.map((r) => r.at)) : null;
    return mockKit.delay({ months: [...months.values()], since: since ? String(Math.floor(since / 1000)) : null, plays: counted.length }, 40);
  },
  period: (p) => mockKit.delay(stats({ year: p.year, month: p.month }), 90),
  async playCounts(ids) {
    const out: Record<string, number> = {};
    for (const r of log) if (r.counted && ids.includes(r.id)) out[r.id] = (out[r.id] ?? 0) + 1;
    return out;
  },
  async clear() {
    const n = log.length;
    log.length = 0;
    subs.forEach((h) => h());
    return n;
  },
  async subscribe(h) {
    subs.add(h);
    return () => subs.delete(h);
  },
};
