// The mock of details (lane E) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts. Songs in the queue have a cached file and downloaded songs
// their download; others have none. Estimates take a moment and one in
// five songs is too uncertain to name a key.
import type { DetailsApi } from "../features/details";
import { mockKit } from "../mock";
import type * as T from "../types";

const hash = (s: string) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);
const KEYS = ["C major", "A minor", "G major", "E minor", "D major", "F♯ minor", "B♭ major", "C♯ minor", "E♭ major", "F major"];
const done = new Map<string, T.AudioAnalysis>();
let running: { id: string; stop: () => void } | null = null;

async function file(id: string): Promise<T.SongFile | null> {
  const { mockApi } = await import("../mock");
  const dl = (await mockApi.downloadsSnapshot()).items.find((i) => i.track.id === id && i.state === "ready");
  const t = mockKit.track(id);
  const bytes = Math.round((t.duration ?? 200) * 16_000);
  if (dl) return { downloaded: true, format: "Opus", kbps: 160, bytes: Math.round(bytes * 1.25) };
  if (mockKit.player.entries.some((e) => e.track.id === id)) return { downloaded: false, format: "AAC", kbps: 128, bytes };
  return null;
}

export const mockDetails: DetailsApi = {
  async track(t) {
    const f = await file(t.id);
    const { mockLastfm } = await import("./lastfm");
    const [fm] = await mockLastfm.plays([t]);
    return mockKit.delay({ file: f, plays: hash(t.id) % 23, lastfmPlays: fm, analysis: done.get(t.id) ?? null, canAnalyse: !!f && !done.has(t.id) }, 90);
  },
  analyse(id) {
    running?.stop();
    return new Promise((resolve, reject) => {
      const h = hash(id);
      const timer = setTimeout(() => {
        running = null;
        const a: T.AudioAnalysis = { bpm: 72 + (h % 1000) / 10, key: h % 5 ? KEYS[h % KEYS.length] : null, confidence: 0.6 };
        done.set(id, a);
        resolve(structuredClone(a));
      }, 2200);
      running = { id, stop: () => (clearTimeout(timer), reject({ code: "stale", message: "Superseded", retryable: false, action: null })) };
    });
  },
  async cancel() {
    running?.stop();
    running = null;
  },
};
