// The mock of taste (lane D) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
//
// `?blocked` starts with a song and an artist blocked; `?private` starts in
// a private session. Home gets a podcast shelf so "Hide podcasts" shows.
import * as M from "../mockdata";
import { mockApi, mockKit } from "../mock";
import type { TasteApi } from "../features/taste";
import type * as T from "../types";
import { mockPlayer } from "./playback";

const q = new URLSearchParams(location.search);
const now = () => Math.floor(Date.now() / 1000);
const PRIVATE_FOR = 3600;

let blocked: T.Blocked[] = [];
let hidePodcasts = false;
let privateTimer: ReturnType<typeof setTimeout> | undefined;
const subs = new Set<(s: T.TasteState) => void>();

const state = (): T.TasteState => ({ blocked: structuredClone(blocked), hidePodcasts });
const emit = () => subs.forEach((h) => h(state()));

if (q.has("blocked")) {
  setTimeout(() => {
    const t = mockKit.track(`mock:${M.tracks[5].id}`);
    const a = M.artists[3];
    blocked = [
      { kind: "track", target: t.id, label: t.title, detail: t.artists.map((x) => x.name).join(", "), created: String(now() - 86400) },
      { kind: "artist", target: `mock:${a.id}`, label: a.name, detail: null, created: String(now() - 3 * 86400) },
    ];
  });
}

function setPrivate(on: boolean) {
  clearTimeout(privateTimer);
  mockPlayer.privateUntil = on ? Date.now() + PRIVATE_FOR * 1000 : null;
  if (on) privateTimer = setTimeout(() => setPrivate(false), PRIVATE_FOR * 1000);
  mockKit.publish();
}
if (q.has("private")) setTimeout(() => setPrivate(true), 50);

const blocks = (t: T.Track) =>
  blocked.some((b) => (b.kind === "track" ? b.target === t.id || b.target === t.counterpart : t.artists.some((a) => a.id === b.target)));

export const mockTaste: TasteApi = {
  state: () => mockKit.delay(state(), 20),
  async block(kind, target, label, detail) {
    blocked = [{ kind, target, label, detail, created: String(now()) }, ...blocked.filter((b) => !(b.kind === kind && b.target === target))];
    emit();
    // As the player's admission_changed: blocked songs leave Up next, the one playing goes at once.
    const P = mockKit.player;
    if (P.index != null) {
      const keep = P.entries.filter((e, i) => i <= P.index! || !blocks(e.track));
      if (keep.length !== P.entries.length) {
        P.entries = keep;
        const uids = new Set(keep.map((e) => e.uid));
        P.canon = P.canon.filter((u) => uids.has(u));
        mockKit.publish(true);
      }
    }
    const cur = mockKit.snapshot().current;
    if (cur && P.playing && blocks(cur.track)) await mockApi.next();
    return mockKit.delay(state(), 60);
  },
  async unblock(kind, target) {
    blocked = blocked.filter((b) => !(b.kind === kind && b.target === target));
    emit();
    return mockKit.delay(state(), 60);
  },
  async setHidePodcasts(on) {
    hidePodcasts = on;
    emit();
    return mockKit.delay(state(), 40);
  },
  async setPrivate(on) {
    setPrivate(on);
    return mockKit.delay(null, 40);
  },
  async subscribe(h) {
    subs.add(h);
    return () => subs.delete(h);
  },
};

/** Whether the simulated history should leave a play out (D36). */
export const mockPrivate = () => mockPlayer.privateUntil !== null;

/** A shelf of podcast episodes for the mock Home, so hiding them shows. */
export function mockPodcastShelf(): T.Shelf {
  const shows = [
    ["Liner Notes", "The stories behind the records", 2810],
    ["Second Verse", "Songwriters on the songs that got away", 3540],
    ["Low End Theory Hour", "Bass, rhythm and the people who play them", 2205],
    ["Night Shift Radio", "Long-form mixes for late work", 4380],
  ] as const;
  const items = shows.map(([title, show, dur], i): T.ShelfItem => {
    const base = mockKit.track(`mock:${M.tracks[(21 + i * 4) % M.tracks.length].id}`);
    return { type: "track", ...base, id: `mock:ep${i}`, title, artists: [{ name: show, id: null }], album: null, duration: dur, kind: "episode", plays: null, liked: null, counterpart: null };
  });
  return { title: "Podcasts for you", subtitle: "New episodes", layout: "carousel", items, more: null };
}
