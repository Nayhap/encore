// Mock backend for `bun run dev` in a plain browser: the invented catalogue
// from mockdata.ts and a simulated player clock, behind the same API as Tauri.
import * as M from "./mockdata";
import type { Api, Handlers } from "./index";
import type * as T from "./types";
import { mockFeatures } from "./mock/index";
import { mockPlayer } from "./mock/playback";
import { mockStatsPlayed } from "./mock/stats";
import { mockPodcastShelf, mockPrivate } from "./mock/taste";

const art = (a: M.Art): T.ArtRef => ({ key: `mock:${a.seed}:${a.motif}`, h1: a.h1, h2: a.h2 });
const mid = (id: string) => `mock:${id}`;
const raw = (id: string) => id.replace(/^mock:/, "");

function toTrack(t: M.MTrack): T.Track {
  const a = M.artists.find((x) => x.id === t.artistId)!;
  const c = M.collections.find((x) => x.id === t.albumId)!;
  return {
    id: mid(t.id),
    title: t.title,
    artists: [{ name: a.name, id: mid(a.id) }],
    album: { name: c.title, id: mid(c.id) },
    duration: t.dur,
    explicit: !!t.explicit,
    art: art(c.art),
    kind: "song",
    plays: t.plays ? `${t.plays} plays` : null,
    liked: liked.has(mid(t.id)),
    playable: true,
    counterpart: null,
  };
}
// Songs with a play count have a music video too (`<id>-mv`, 12 s of intro).
const MV = "-mv";
const INTRO = 12;
function toVideo(t: M.MTrack): T.Track {
  return { ...toTrack(t), id: mid(t.id + MV), album: null, duration: t.dur + INTRO, kind: "video", plays: t.plays ? `${t.plays} views` : null, liked: liked.has(mid(t.id + MV)), counterpart: null };
}
function track(id: string): T.Track {
  const r = raw(id);
  if (r.endsWith(MV)) return toVideo(M.tracks.find((t) => t.id === r.slice(0, -MV.length))!);
  return toTrack(M.tracks.find((t) => t.id === r)!);
}
function counterpartOf(t: T.Track): T.Counterpart | null {
  const r = raw(t.id);
  const songId = r.endsWith(MV) ? r.slice(0, -MV.length) : r;
  const m = M.tracks.find((x) => x.id === songId);
  if (!m?.plays) return null;
  if (r.endsWith(MV)) return { of: t.id, track: { ...toTrack(m), counterpart: t.id }, segments: [{ fromStart: INTRO, toStart: 0, duration: m.dur }] };
  return { of: t.id, track: { ...toVideo(m), counterpart: t.id }, segments: [{ fromStart: 0, toStart: INTRO, duration: m.dur }] };
}

function kindOf(c: M.MCollection): T.CollectionKind {
  return c.kind === "album" ? "album" : c.kind === "single" ? "single" : c.kind === "mix" ? "mix" : c.id === "pl-liked" ? "liked" : "playlist";
}
function card(c: M.MCollection): T.CollectionCard {
  return {
    id: mid(c.id),
    kind: kindOf(c),
    title: c.title,
    subtitle: c.year ? `${c.kind === "album" ? "Album" : c.kind === "single" ? "Single" : "Playlist"} • ${c.by} • ${c.year}` : `${c.kind === "mix" ? "Mix" : "Playlist"} • ${c.by}`,
    artists: c.artistId ? [{ name: c.by, id: mid(c.artistId) }] : [],
    year: c.year ?? null,
    art: art(c.art),
    playId: null,
  };
}
function artistCard(a: M.MArtist): T.ArtistCard {
  return { id: mid(a.id), name: a.name, subtitle: `Artist • ${a.subs} subscribers`, art: art(a.art) };
}
const item = {
  t: (t: M.MTrack): T.ShelfItem => ({ type: "track", ...toTrack(t) }),
  c: (c: M.MCollection): T.ShelfItem => ({ type: "collection", ...card(c) }),
  a: (a: M.MArtist): T.ShelfItem => ({ type: "artist", ...artistCard(a) }),
};
const shelf = (title: string, items: T.ShelfItem[], layout: "carousel" | "list" | "grid" = "carousel", subtitle: string | null = null): T.Shelf => ({ title, subtitle, layout, items, more: null });

// ---------- the catalogue for library tools (lane B, D34, D35) ----------
// A long description ending in YouTube's own Wikipedia credit, as many
// album descriptions do, to show the folded description and its credit line.
const LONG_DESC =
  "The long way round is a running mix of everything in this library, kept in no particular order. It started as a single afternoon of shuffling and grew a song at a time: a night drive here, a rainy commute there, the odd record found in a friend's car. There is no theme, no tempo arc and no rule beyond keeping what still sounds right the next morning. It is long on purpose, to be left playing while you do something else, and to turn up a song you had forgotten you liked. Some entries are albums played in full, some are single tracks pulled from compilations, and a few are live takes that never made it onto a record. From Wikipedia (https://en.wikipedia.org/wiki/Shuffle_play) under Creative Commons Attribution CC-BY-SA 3.0 (https://creativecommons.org/licenses/by-sa/3.0/legalcode)";

// "Everything twice" (yours) has exact copies and songs next to their music
// videos, for Duplicates; "The long way round" has 1,340 rows that arrive
// 100 at a time, for "Searching 500 of 1,340 · Load all".
{
  const ids = M.tracks.map((t) => t.id);
  const pick = (n: number, step: number, from = 0) => Array.from({ length: n }, (_, i) => ids[(from + i * step) % ids.length]);
  const twice = [...pick(24, 1)];
  twice.splice(6, 0, ids[2]);
  twice.splice(13, 0, ids[0] + MV, ids[9]);
  twice.push(ids[4], ids[14] + MV, ids[2]);
  M.collections.push(
    { id: "pl-twice", kind: "playlist", title: "Everything twice", by: "You", art: { seed: 230, h1: 160, h2: 280, motif: "stripes" }, trackIds: twice, desc: "Some songs are in here more than once." },
    { id: "pl-long", kind: "playlist", title: "The long way round", by: "Encore Radio", art: { seed: 231, h1: 25, h2: 200, motif: "wave" }, trackIds: pick(1340, 7, 3), desc: LONG_DESC },
    { id: "pl-wordy", kind: "playlist", title: "songs for the night bus home when the windows fog up and the city lights go soft and everyone on board is half asleep except you", by: "You", art: { seed: 232, h1: 250, h2: 320, motif: "orbit" }, trackIds: pick(9, 5, 1), desc: "Started this on a winter tour.\n\nRules: nothing over five minutes, nothing with a drop, nothing I would skip at 2 am. Add yours, but keep it quiet.\n\nLast tidied in March. If a song vanished, YouTube took it down, not me." },
  );
}
// Rows per page of a long mock playlist.
const PAGE = 100;
// Each artist's singles and EPs: their album's songs, one or two at a time.
const singles: M.MCollection[] = M.collections
  .filter((c) => c.kind === "album")
  .flatMap((a) =>
    a.trackIds.slice(0, 3).map((t, i): M.MCollection => ({
      id: `sg-${t}`, kind: "single", title: `${M.tracks.find((x) => x.id === t)!.title}${i === 2 ? " (Remixes)" : ""}`, by: a.by, artistId: a.artistId,
      year: (a.year ?? 2024) - i, art: { ...a.art, seed: a.art.seed + 40 + i, h1: (a.art.h1 + 40 * (i + 1)) % 360 }, trackIds: i === 2 ? a.trackIds.slice(0, 2) : [t],
    })),
  );
const anyCollection = (id: string) => M.collections.find((x) => x.id === id) ?? singles.find((x) => x.id === id) ?? community(id);
// Playlists other listeners made, as the community-playlist search finds them (D35).
const owners = ["nightbus", "Mara K.", "lofi archive", "Theo", "city lights radio", "june"];
function communityFor(a: M.MArtist): M.MCollection[] {
  const songs = M.tracks.filter((t) => t.artistId === a.id).map((t) => t.id);
  const others = M.tracks.filter((t) => t.artistId !== a.id).map((t) => t.id);
  return [`${a.name} essentials`, `best of ${a.name}`, `${a.name} + friends`, "late night drive", "songs for the train home", `${a.name.toLowerCase()} deep cuts`].map((title, i) => ({
    id: `cp-${a.id}-${i}`, kind: "playlist", title, by: owners[i % owners.length], art: { seed: 400 + i * 7 + a.art.seed, h1: (a.art.h1 + i * 50) % 360, h2: (a.art.h2 + i * 30) % 360, motif: (["orbit", "grid", "wave", "sun", "blocks", "stripes"] as const)[i] },
    trackIds: [...songs, ...others.slice(i * 3, i * 3 + 6)],
  }));
}
function community(id: string) {
  const m = id.match(/^cp-(.+)-\d+$/);
  const a = m && M.artists.find((x) => x.id === m[1]);
  return a ? communityFor(a).find((c) => c.id === id) : undefined;
}
/** The mock's catalogue, for ./mock/libraryTools.ts. */
export const mockCatalogue = {
  singlesOf: (artistId: string) => singles.filter((s) => s.artistId === artistId).map(card),
  albumsOf: (artistId: string) => M.collections.filter((c) => c.kind === "album" && c.artistId === artistId).map(card),
  year: (albumId: string) => anyCollection(raw(albumId))?.year ?? null,
};

const liked = new Set<string>(M.collections.find((c) => c.id === "pl-liked")!.trackIds.map(mid));
const delay =<T>(v: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(structuredClone(v)), ms));

// ---------- your playlists and history ----------

const privacy = new Map<string, T.Privacy>([["pl-late", "private"], ["pl-jp", "public"], ["pl-rain", "unlisted"]]);
let setN = 0;
const setIds = new Map<string, string[]>();
function setsOf(c: M.MCollection) {
  let s = setIds.get(c.id);
  if (!s || s.length !== c.trackIds.length) {
    s = c.trackIds.map(() => `sv${++setN}`);
    setIds.set(c.id, s);
  }
  return s;
}
const mine = (id: string) => M.collections.find((c) => c.id === raw(id).replace(/^VL/, "") && c.by === "You");
const changed = (id: string | null) => h?.playlists({ playlistId: id ? mid(id) : null });

let rowN = 0;
const row = (id: string): T.HistoryRow => ({ rowId: `h${++rowN}`, track: track(id), removable: true });
const hist: { title: string; rows: T.HistoryRow[] }[] = [
  { title: "Today", rows: [] },
  { title: "Yesterday", rows: M.tracks.slice(8, 12).map((t) => row(t.id)) },
  { title: "Last week", rows: [...M.tracks.slice(20, 23).map((t) => row(t.id)), row(M.tracks[0].id + MV), ...M.tracks.slice(30, 33).map((t) => row(t.id))] },
];
hist[0].rows = [M.tracks[3], M.tracks[14], M.tracks[0]].map((t) => row(t.id));
let reportHistory = true;
let normalise = true;
function played(t: T.Track) {
  // The local play log counts every play (D30); history skips a private session's (D36).
  mockStatsPlayed(t);
  if (!reportHistory || auth.state !== "signedIn" || mockPrivate()) return;
  hist[0].rows.unshift({ rowId: `h${++rowN}`, track: t, removable: true });
  h?.history({ trackId: t.id });
}


// ---------- simulated player ----------

let h: Handlers | null = null;
let rev = 0;
let qrev = 0;
let tgen = 0;
let sgen = 0;
const P = {
  entries: [] as T.QueueEntry[],
  canon: [] as string[],
  index: null as number | null,
  position: 47,
  playing: false,
  shuffle: false,
  repeat: "off" as T.Repeat,
  autoplay: true,
  volume: 0.7,
  muted: false,
  source: null as T.QueueSource | null,
  phase: "idle" as T.Phase,
  error: null as T.PlayerSnapshot["error"],
};
let uidN = 0;
const uid = () => `m${++uidN}`;
let lastEntries: T.QueueEntry[] = [];
let lastQrev = 0;

function snap(): T.PlayerSnapshot {
  const cur = P.index != null ? P.entries[P.index] ?? null : null;
  return {
    revision: String(++rev), trackGen: String(tgen), seekGen: String(sgen), phase: P.phase, playing: P.playing,
    current: cur, index: P.index, position: P.position, duration: cur?.track.duration ?? null,
    shuffle: P.shuffle, repeat: P.repeat, autoplay: P.autoplay, volume: P.volume, muted: P.muted, source: P.source,
    stream: cur ? { itag: 0, codec: "mock", kbps: null, fromCache: true, seekable: true, authenticated: false } : null,
    error: P.error, skippedInARow: 0, queueLoading: false, noOutput: false,
    loop: mockPlayer.loop, sleep: mockPlayer.sleep, private: mockPlayer.private,
  };
}
function publish(queueChanged = false) {
  if (!h) return;
  if (queueChanged) {
    qrev++;
    h.queue({ base: String(lastQrev), revision: String(qrev), start: 0, delete: lastEntries.length, insert: P.entries, index: P.index });
    lastEntries = [...P.entries];
    lastQrev = qrev;
  }
  h.playerState(snap());
}
function anchor() {
  const cur = P.index != null ? P.entries[P.index] : null;
  h?.playerPosition({ trackGen: String(tgen), seekGen: String(sgen), position: P.position, duration: cur?.track.duration ?? null, playing: P.playing });
}
function shuffled<X>(xs: X[]) {
  const a = [...xs];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}
function go(i: number, at = 0) {
  // Offline, like the Rust player: pass over songs that aren't downloaded.
  if (!dl.online && !isLocal(P.entries[i]?.track.id)) {
    const n = P.entries.length;
    const order = [...Array(n).keys()].map((k) => (i + k) % n).filter((j) => j >= i || P.repeat === "all");
    const j = order.find((x) => isLocal(P.entries[x].track.id));
    if (j == null) {
      P.index = i;
      P.playing = false;
      P.phase = "error";
      P.error = { kind: "network", message: "You're offline, and nothing else in the queue is downloaded." };
      publish(true);
      h?.notice({ text: P.error.message, level: "warn", action: null });
      return;
    }
    const passed = (j - i + n) % n;
    h?.notice({ text: passed === 1 ? `Skipped “${P.entries[i].track.title}”: it isn't downloaded.` : `Skipped ${passed} songs that aren't downloaded.`, level: "warn", action: null });
    i = j;
  }
  P.error = null;
  if (P.index !== i) endLoop();
  P.index = i;
  P.position = at;
  noteCycle();
  P.playing = true;
  P.phase = "playing";
  tgen++;
  publish(true);
  anchor();
  sendLyrics();
  const t = P.entries[i]?.track;
  if (t) setTimeout(() => P.playing && P.entries[P.index ?? -1]?.track.id === t.id && played(t), 2000);
}
function sendLyrics() {
  const cur = P.index != null ? P.entries[P.index] : null;
  if (!cur || !h) return;
  void mockApi.lyrics(cur.track).then((l) => h?.lyrics({ trackId: cur.track.id, lyrics: l }));
}
function extendRadio() {
  const have = new Set(P.entries.map((e) => e.track.id));
  const pool = shuffled(M.tracks.filter((t) => !have.has(mid(t.id)))).slice(0, 5).map(toTrack);
  for (const t of pool) {
    const e = { uid: uid(), track: t, origin: "radio" as const };
    P.entries.push(e);
    P.canon.push(e.uid);
  }
  return pool.length;
}
function ended() {
  if (P.index == null) return;
  const sl = mockPlayer.sleepState.mode;
  const last = P.index >= P.entries.length - 1;
  if (sl === "endOfSong" || (sl === "endOfQueue" && last)) {
    // The sleep timer waited for this song: the next one waits, paused (D26).
    mockPlayer.sleepState.mode = null;
    P.playing = false;
    endLoop();
    if (sl === "endOfSong" && !last) {
      P.index++;
      P.position = 0;
      P.phase = "paused";
      tgen++;
      sendLyrics();
    } else P.phase = "ended";
    publish(true);
    anchor();
    h?.notice({ text: "Paused by the sleep timer.", level: "info", action: null });
    return;
  }
  if (P.repeat === "one") {
    P.position = 0;
    sgen++;
    anchor();
  } else if (P.index < P.entries.length - 1) go(P.index + 1);
  else if (P.repeat === "all") go(0);
  else if (P.autoplay && mockPlayer.sleepState.mode !== "endOfQueue" && extendRadio()) go(P.index + 1);
  else {
    P.playing = false;
    P.phase = "ended";
    publish();
  }
}
let last = performance.now();
setInterval(() => {
  const now = performance.now();
  const dt = (now - last) / 1000;
  last = now;
  const cur = P.index != null ? P.entries[P.index] : null;
  const sl = mockPlayer.sleepState;
  if (sl.mode === "timer" && Date.now() >= sl.deadline) {
    // The Rust engine fades out over the last 10 s first.
    sl.mode = null;
    if (P.playing) {
      P.playing = false;
      P.phase = "paused";
      h?.notice({ text: "Paused by the sleep timer.", level: "info", action: null });
    }
    publish();
    anchor();
  }
  if (!P.playing || !cur) return;
  P.position += dt;
  const l = mockPlayer.loop;
  if (l?.a != null && l.b != null && P.position >= l.b) {
    // Wraps at B, like the engine (D26).
    P.position = l.a + (P.position - l.b);
    anchor();
  }
  if (P.position >= (cur.track.duration ?? 0)) ended();
}, 100);
setInterval(() => P.playing && anchor(), 500);

const refuse = (message: string) => ({ code: "invalid", message, retryable: false, action: null });

/** End the loop on a song change, putting repeat-one back. */
function endLoop() {
  if (!mockPlayer.loop) return;
  mockPlayer.loop = null;
  if (mockPlayer.loopRepeat) P.repeat = mockPlayer.loopRepeat;
  mockPlayer.loopRepeat = null;
}

export function simLoop(a: number, b: number) {
  const cur = P.index != null ? P.entries[P.index] : null;
  const dur = cur?.track.duration;
  if (!cur || P.phase === "idle") throw refuse("Play the song to loop part of it.");
  if (!dur) throw refuse("This song's length isn't known yet. Try again in a moment.");
  [a, b] = a <= b ? [a, b] : [b, a];
  a = Math.max(0, Math.min(a, dur));
  b = Math.max(0, Math.min(b, dur));
  if (b - a < 1) throw refuse("A loop needs to be at least a second long.");
  if (!mockPlayer.loop && P.repeat === "one") {
    mockPlayer.loopRepeat = "one";
    P.repeat = "off";
  }
  mockPlayer.loop = { a, b };
  if (P.position < a - 0.05 || P.position >= b) {
    P.position = a;
    sgen++;
  }
  publish();
  anchor();
}

export function simClearLoop() {
  endLoop();
  publish();
  anchor();
}

export function simSleep(mode: T.SleepMode, minutes: number | null) {
  if (mode === "endOfSong" && mockPlayer.loop) throw refuse("Clear the loop to stop at the end of this song.");
  if (mode === "timer" && !(minutes && minutes > 0)) throw refuse("Choose how long until Encore pauses.");
  mockPlayer.sleepState.mode = mode;
  mockPlayer.sleepState.deadline = Date.now() + (minutes ?? 0) * 60_000;
  publish();
}

export function simCancelSleep() {
  mockPlayer.sleepState.mode = null;
  publish();
}

/** Shuffle cycles (D33), per collection for this session: the songs
 *  played in the current cycle. */
const cycles = new Map<string, Set<string>>();
function noteCycle() {
  const id = P.source?.collectionId;
  const e = P.index != null ? P.entries[P.index] : null;
  if (!P.shuffle || !id || !e || e.origin !== "collection") return;
  const c = cycles.get(id) ?? new Set();
  c.add(e.track.id);
  const members = new Set(P.entries.filter((x) => x.origin === "collection").map((x) => x.track.id));
  cycles.set(id, c.size >= members.size ? new Set() : c);
}

/** A collection shuffled in its cycle: each song once, the ones not yet
 *  played in the cycle first. */
function replaceCycle(tracks: T.Track[], source: T.QueueSource) {
  const seen = new Set<string>();
  tracks = tracks.filter((t) => !seen.has(t.id) && seen.add(t.id));
  const played = cycles.get(source.collectionId!) ?? new Set();
  const fresh = shuffled(tracks.filter((t) => !played.has(t.id)));
  const stale = shuffled(tracks.filter((t) => played.has(t.id)));
  const es = [...fresh, ...stale].map((t) => ({ uid: uid(), track: t, origin: "collection" as const }));
  const rank = new Map(tracks.map((t, i) => [t.id, i]));
  P.canon = [...es].sort((x, y) => rank.get(x.track.id)! - rank.get(y.track.id)!).map((e) => e.uid);
  P.entries = es;
  P.shuffle = true;
  P.source = source;
  go(0);
}

function replace(tracks: T.Track[], start: number, source: T.QueueSource | null, shuffle: boolean) {
  let es = tracks.map((t) => ({ uid: uid(), track: t, origin: "collection" as const }));
  P.canon = es.map((e) => e.uid);
  if (shuffle) {
    const first = es.splice(start, 1)[0];
    es = [first, ...shuffled(es)];
    start = 0;
  }
  P.entries = es;
  P.shuffle = shuffle;
  P.source = source;
  go(start);
}

// Start with a queue loaded but paused, like reopening the app (`?empty`
// starts with nothing queued, like a fresh install).
if (!new URLSearchParams(location.search).has("empty")) {
  const c = M.collections.find((x) => x.id === "pl-late")!;
  P.entries = c.trackIds.map((id) => ({ uid: uid(), track: track(id), origin: "collection" as const }));
  P.canon = P.entries.map((e) => e.uid);
  P.index = 0;
  P.phase = "paused";
  P.source = { label: c.title, collectionId: mid(c.id), radioPlaylist: null };
  // `?loop`, `?sleep`, `?autoplayoff` (mock/playback.ts).
  const q = new URLSearchParams(location.search);
  if (q.has("loop")) mockPlayer.loop = { a: 41, b: 63 };
  if (q.has("sleep")) Object.assign(mockPlayer.sleepState, { mode: "timer", deadline: Date.now() + 23 * 60_000 + 12_000 });
  if (q.has("autoplayoff")) P.autoplay = false;
}

// ---------- player engine (D37) ----------
// Built in: nothing to set up. `?needsupdate` shows the notice Settings
// gives when YouTube changed something only an update fixes.
const q = new URLSearchParams(location.search);
const engine = { player: "4efb8b2c/main" as string | null, notice: q.has("needsupdate") ? "YouTube changed how its songs play, and this version of Encore can't follow yet. Check for a newer one under Updates, below." : null };
function engineStatus(): T.EngineStatus {
  return { solver: "yt-dlp-ejs 0.8.0", engine: "quickjs-ng 0.16.2", extractor: "yt-dlp 2026.10.26 (b77c930)", player: engine.player, notice: engine.notice };
}

// ---------- updates (D24) ----------
// An automatic check runs 3 s after start. `?update` finds Encore 0.5.0 and
// downloads it over 6 s; `?update-manual` has automatic updates off, so it
// only says so (Download on request); `?update-deb` is a .deb install, which
// links to the release page, as does `?update-rpm` for an .rpm install;
// `?update-fail` makes checks fail;
// `?update-translocated` runs from a translocated copy on macOS; `?updated`
// starts just after an update, with its "What's new".
const UPDATE_NOTES = `## What's new

- **In-app updates.** Encore checks for new versions a few times a day, downloads them in the background and installs them the next time you quit or restart it.
- **Settings → Updates** shows the version, checks on request and can turn automatic updates off.
- Press <kbd>⌘K</kbd> and choose **Check for updates** from anywhere.

## Fixes

- Quitting saves the queue and position at once, instead of up to 5 seconds earlier.`;
const updateInfo = (version: string): T.UpdateInfo => ({
  version,
  date: new Date(Date.now() - 2 * 86400_000).toISOString(),
  notes: UPDATE_NOTES.split("\n").filter((l) => l.startsWith("- ")).map((l) => l.slice(2).replace(/\*\*|<\/?kbd>/g, "")),
  page: `https://github.com/Nayhap/encore/releases/tag/v${version}`,
});
const upFound = q.has("update") || q.has("update-manual") || q.has("update-deb") || q.has("update-rpm") || q.has("update-translocated");
const up = {
  rev: 1,
  current: "0.4.0",
  auto: !q.has("update-manual"),
  install: (q.has("update-deb") ? "deb" : q.has("update-rpm") ? "rpm" : "app") as T.InstallKind,
  reason: q.has("update-deb")
    ? "You installed Encore from the .deb package, which Encore can't update itself. Download the new .deb from the releases page."
    : q.has("update-rpm")
      ? "You installed Encore from the .rpm package, which Encore can't update itself. Download the new .rpm from the releases page."
      : q.has("update-translocated")
        ? "Move Encore to your Applications folder, then open it from there to let it update itself."
        : null,
  phase: "idle" as T.UpdatePhase,
  available: null as T.UpdateInfo | null,
  received: 0,
  total: 0,
  error: null as string | null,
  checkedAt: null as number | null,
  skipped: null as string | null,
  requested: false,
  timer: 0,
  updated: q.has("updated") ? updateInfo("0.4.0") : null,
};
function upStatus(): T.UpdateStatus {
  const dl = up.phase === "downloading";
  return {
    revision: String(up.rev),
    current: up.current,
    install: up.install,
    canInstall: !up.reason,
    reason: up.reason,
    auto: up.auto,
    phase: up.phase,
    available: up.available,
    received: dl ? up.received : null,
    total: dl ? up.total : null,
    error: up.error,
    checkedAt: up.checkedAt,
    skipped: up.skipped,
    atQuit: up.phase === "ready" && (up.auto || up.requested),
    updated: up.updated,
  };
}
const upEmit = () => {
  up.rev++;
  h?.update(upStatus());
};
async function upCheck(manual: boolean) {
  if (up.phase === "downloading" || up.phase === "installing") return upStatus();
  if (up.phase !== "ready") up.phase = "checking";
  upEmit();
  await new Promise((r) => setTimeout(r, 900));
  if (q.has("update-fail")) {
    up.error = "GitHub didn't answer";
    if (up.phase === "checking") up.phase = up.available ? "available" : "failed";
  } else {
    up.error = null;
    up.checkedAt = Date.now() / 1000;
    if (up.phase === "ready") return (upEmit(), upStatus());
    if (upFound) {
      up.available = updateInfo("0.5.0");
      up.phase = "available";
      if (up.auto && !up.reason && up.skipped !== "0.5.0") upDownload(false);
    } else {
      up.available = null;
      up.phase = "upToDate";
    }
  }
  void manual;
  upEmit();
  return upStatus();
}
function upDownload(requested: boolean) {
  if (!up.available || up.reason || !["available", "failed"].includes(up.phase)) return;
  up.phase = "downloading";
  up.requested = requested;
  up.received = 0;
  up.total = 28_400_000;
  clearInterval(up.timer);
  up.timer = window.setInterval(() => {
    if (up.phase !== "downloading") return clearInterval(up.timer);
    up.received = Math.min(up.total, up.received + up.total / 24);
    if (up.received >= up.total) {
      clearInterval(up.timer);
      up.phase = "ready";
    }
    upEmit();
  }, 250);
  upEmit();
}
setTimeout(() => void upCheck(false), 3000);

// ---------- downloads (D23) ----------
// One simulated worker: a song "downloads" in 1.5 s, at 0.25 s ticks.
// `?downloaded` starts with "Late night" kept and downloaded, plus one song
// on its own; `?offline` starts offline (the browser's online and offline
// events flip it); `?dlfail` fails every third song as unavailable, and
// the fifth song meets a bot check that pauses everything for 20 s, as the
// real worker does.
const dl = {
  rev: 1,
  ver: 0,
  items: new Map<string, T.DownloadItem>(),
  pins: new Set<string>(),
  kept: new Map<string, { card: T.CollectionCard; ids: string[]; paused: boolean; synced: number }>(),
  paused: false,
  online: !q.has("offline"),
  active: null as null | { id: string; bytes: number; total: number; attempt: string },
  started: 0,
  botUntil: 0,
};
const now = () => Date.now() / 1000;
const isLocal = (id: string | undefined) => !!id && dl.items.get(id)?.state === "ready";
const songBytes = (t: T.Track) => Math.round((t.duration ?? 200) * 16_000);
const keptBy = (id: string) => [...dl.kept.values()].find((k) => k.ids.includes(id));
const wanted = (id: string) => dl.pins.has(id) || [...dl.kept.values()].some((k) => !k.paused && k.ids.includes(id));

function dlSet(id: string, state: T.DownloadState, extra: Partial<T.DownloadItem> = {}) {
  const old = dl.items.get(id);
  const item: T.DownloadItem = {
    track: { ...track(id), liked: null }, state, error: null, bytes: 0, codec: null, kbps: null, itag: null,
    added: old?.added ?? now(), version: String(++dl.ver), attempt: old?.attempt ?? "0", ...extra,
  };
  if (state === "removed") dl.items.delete(id);
  else dl.items.set(id, item);
  dl.rev++;
  h?.downloadItem({ revision: String(dl.rev), item });
}
function dlSummary(): T.DownloadsSummary {
  const all = [...dl.items.values()];
  const ready = all.filter((i) => i.state === "ready");
  return {
    revision: String(dl.rev),
    available: true,
    songs: ready.length,
    waiting: all.filter((i) => i.state === "queued" || i.state === "downloading").length,
    failed: all.filter((i) => i.state === "failed").length,
    bytes: ready.reduce((n, i) => n + (i.bytes ?? 0), 0) + (dl.active?.bytes ?? 0),
    active: dl.active ? { trackId: dl.active.id, attempt: dl.active.attempt, bytes: dl.active.bytes, total: dl.active.total } : null,
    paused: dl.paused,
    ...(!dl.online
      ? { wait: "offline" as const, message: "You're offline. Downloads continue when you're back online." }
      : Date.now() < dl.botUntil
        ? { wait: "botCheck" as const, message: `YouTube asked Encore to prove it isn't a bot. Downloads continue in 1 minute.` }
        : { wait: null, message: null }),
    online: dl.online,
    syncing: [],
  };
}
function dlKept(): T.KeptCollection[] {
  return [...dl.kept.values()].map((k) => {
    const items = k.ids.map((id) => dl.items.get(id));
    return {
      card: k.card, account: "mock", channel: auth.state === "signedIn" ? auth.channel.name : null, mine: true, songs: k.ids.length,
      ready: items.filter((i) => i?.state === "ready").length, failed: items.filter((i) => i?.state === "failed").length,
      bytes: items.reduce((n, i) => n + (i?.state === "ready" ? (i.bytes ?? 0) : 0), 0), synced: k.synced, error: null, gone: false, paused: k.paused,
    };
  });
}
const dlEmit = () => h?.downloads(dlSummary());
/** Kept collections changed: a newer revision makes the UI take a snapshot. */
function dlChanged() {
  dl.rev++;
  dlEmit();
}
setInterval(() => {
  if (dl.paused || !dl.online) return;
  if (dl.botUntil && Date.now() >= dl.botUntil) {
    dl.botUntil = 0;
    dlEmit();
  }
  if (Date.now() < dl.botUntil) return;
  if (!dl.active) {
    const next = [...dl.items.values()].find((i) => i.state === "queued" && wanted(i.track.id));
    if (!next) return;
    const id = next.track.id;
    if (q.has("dlfail")) {
      const n = ++dl.started;
      // A bot check waits and keeps the song queued; it never fails it.
      if (n === 5) {
        dl.botUntil = Date.now() + 20_000;
        return dlEmit();
      }
      if (n % 3 === 0) {
        dlSet(id, "failed", { error: "This song isn't available in your country." });
        return dlEmit();
      }
    }
    dl.active = { id, bytes: 0, total: songBytes(next.track), attempt: String(Date.now()) };
    dlSet(id, "downloading", { attempt: dl.active.attempt });
    return dlEmit();
  }
  const a = dl.active;
  a.bytes = Math.min(a.total, a.bytes + a.total / 6);
  if (a.bytes >= a.total) {
    dl.active = null;
    dlSet(a.id, "ready", { bytes: a.total, codec: "aac", kbps: 128, itag: 140, attempt: a.attempt });
  }
  dlEmit();
}, 250);
function setOnline(on: boolean) {
  if (dl.online === on) return;
  dl.online = on;
  h?.connectivity(on);
  dlEmit();
}
addEventListener("online", () => setOnline(true));
addEventListener("offline", () => setOnline(false));
if (q.has("downloaded")) {
  const c = M.collections.find((x) => x.id === "pl-late")!;
  dl.kept.set(mid(c.id), { card: card(c), ids: c.trackIds.map(mid), paused: false, synced: now() - 3600 });
  for (const id of [...c.trackIds.map(mid), mid(M.tracks[20].id)]) dlSet(id, "ready", { bytes: songBytes(track(id)), codec: "aac", kbps: 128, itag: 140 });
  dl.pins.add(mid(M.tracks[20].id));
}

const channels: T.AccountChannel[] = [
  { key: "0:primary", name: "Riley Park", handle: "@rileypark", pageId: null, authUser: 0, isBrand: false, photo: null },
  { key: "0:123", name: "Mock Brand Channel", handle: "@mockbrand", pageId: "123", authUser: 0, isBrand: true, photo: null },
];
// `?signedin` starts the mock signed in, for screenshots of the full layout.
let auth: T.AuthState = new URLSearchParams(location.search).has("signedin")
  ? { state: "signedIn", channel: channels[0], channels, source: "edge:Default", premium: null }
  : { state: "anonymous" };

const lyricLines = (t: M.MTrack): T.LyricLine[] | null => (t.lyrics ? t.lyrics.map((l) => ({ t: l.t, end: null, text: l.text, words: null })) : null);

export const mockApi: Api = {
  async subscribe(handlers) {
    h = handlers;
    return () => (h = null);
  },
  async bootstrap() {
    lastEntries = [...P.entries];
    lastQrev = qrev;
    setTimeout(sendLyrics, 50);
    return delay({
      version: "mock",
      reportHistory,
      normalise,
      notice: null,
      auth,
      player: snap(),
      queue: { revision: String(qrev), entries: P.entries, index: P.index },
      engine: engineStatus(),
      online: dl.online,
      update: upStatus(),
    }, 10);
  },
  playerEngine: async () => engineStatus(),
  async playerEngineReset() {
    engine.player = null;
    engine.notice = null;
    return delay(engineStatus(), 200);
  },
  cacheInfo: async () => ({ audioBytes: 0, artBytes: 0, lyricsEntries: 0 }),
  cacheClear: async () => {},
  home: () =>
    delay({
      title: null,
      chips: M.moods.slice(0, 7).map((m) => ({ title: m.name, browseId: "FEmusic_home", params: m.name, selected: false })),
      cursor: null,
      shelves: [
        shelf("Listen again", [item.t(M.tracks[15]), item.a(M.artists[1]), item.c(M.collections[2]), item.t(M.tracks[25]), item.c(M.collections.find((c) => c.id === "pl-jp")!), item.a(M.artists[0]), item.t(M.tracks[20]), item.c(M.collections.find((c) => c.id === "pl-rain")!)], "carousel", "From the last two weeks"),
        shelf("Quick picks", M.tracks.filter((t) => t.plays).slice(0, 12).map(item.t), "list"),
        shelf("Mixed for you", [...M.collections.filter((c) => c.kind === "mix").map(item.c), ...M.artists.slice(0, 4).map(item.a)], "carousel", "Updated daily"),
        shelf("New releases", M.collections.filter((c) => c.kind === "album").map(item.c)),
        mockPodcastShelf(),
      ],
    }),
  explore: () =>
    delay({
      title: null,
      chips: [],
      cursor: null,
      shelves: [
        shelf("", [
          { type: "link", title: "New releases", browseId: "FEmusic_new_releases", params: null, hue: null },
          { type: "link", title: "Charts", browseId: "FEmusic_charts", params: null, hue: null },
          { type: "link", title: "Moods & genres", browseId: "FEmusic_moods_and_genres", params: null, hue: null },
        ], "grid"),
        { ...shelf("New albums & singles", M.collections.filter((c) => c.kind === "album").map(item.c)), more: { title: "New albums & singles", browseId: "FEmusic_new_releases_albums", params: null, hue: null } },
        shelf("Moods & genres", M.moods.map((m) => ({ type: "link" as const, title: m.name, browseId: "FEmusic_moods_and_genres_category", params: m.name, hue: m.h }))),
      ],
    }),
  browsePage: (b, p) =>
    // "Show all" on Explore's new releases: one shelf, albums and singles mixed.
    b === "FEmusic_new_releases_albums"
      ? delay({ title: "New albums & singles", chips: [], cursor: null, shelves: [shelf("", [...M.collections.filter((c) => c.kind === "album").map(item.c), ...singles.map(item.c)])] })
      : b.startsWith("MPAD")
      ? delay({ title: M.artists.find((a) => `MPAD${a.id}` === b)?.name ?? null, chips: [], cursor: null, shelves: [shelf("", (p === "singles" ? singles.filter((c) => `MPAD${c.artistId}` === b) : M.collections.filter((c) => c.kind === "album" && `MPAD${c.artistId}` === b)).map(item.c), "grid")] })
      : delay({
      title: p ?? (b === "FEmusic_charts" ? "Charts" : b === "FEmusic_new_releases" ? "New releases" : "Moods & genres"),
      chips: [],
      cursor: null,
      shelves: [shelf("Playlists", M.collections.filter((c) => c.kind !== "album").map(item.c)), shelf("Albums", M.collections.filter((c) => c.kind === "album").map(item.c))],
    }),
  moreShelves: async () => ({ title: null, chips: [], cursor: null, shelves: [] }),
  async search(q, filter) {
    // The community-playlist search (D35): listeners' playlists for an artist.
    if (filter === "communityPlaylists") {
      const a = M.artists.find((x) => x.name.toLowerCase() === q.trim().toLowerCase());
      return delay({ query: q, top: null, songs: [], videos: [], albums: [], artists: [], playlists: a ? communityFor(a).map(card) : [], didYouMean: null, cursor: null }, 300);
    }
    const norm = (s: string) => s.toLowerCase().normalize("NFKC");
    const s = norm(q.trim());
    const hit = (...xs: string[]) => xs.some((x) => norm(x).includes(s));
    const songs = M.tracks.filter((t) => hit(t.title, M.artists.find((a) => a.id === t.artistId)!.name)).slice(0, 8).map(toTrack);
    const artists = M.artists.filter((a) => hit(a.name)).map(artistCard);
    const cols = M.collections.filter((c) => hit(c.title, c.by)).map(card);
    const top = artists.find((a) => norm(a.name).startsWith(s));
    return delay({
      query: q, top: top ? { type: "artist", ...top } : songs[0] ? { type: "track", ...songs[0] } : null,
      songs, videos: [], albums: cols.filter((c) => c.kind === "album"), artists, playlists: cols.filter((c) => c.kind !== "album"), didYouMean: null, cursor: null,
    }, 60);
  },
  suggestions: async () => [],
  async collection(id) {
    // Offline, a kept collection is its saved copy (D23).
    if (!dl.online) {
      const k = dl.kept.get(id);
      if (!k) throw { code: "network", message: "Couldn't reach YouTube. Check your internet connection.", retryable: true, action: null };
      return delay({ ...k.card, description: null, stats: `${k.ids.length} songs`, trackCount: k.ids.length, entries: k.ids.map((t, i) => ({ entryId: `${t}#${i}`, track: track(t), setVideoId: null, videoId: t })), cursor: null, playlistId: null, saved: null, editable: false, privacy: null, related: [] } as T.Collection);
    }
    if (id === "ytm:VLLM" && auth.state !== "signedIn") throw { code: "authRequired", message: "Sign in to YouTube Music to use this.", retryable: false, action: "signIn" };
    const c = id === "ytm:VLLM" ? M.collections.find((x) => x.id === "pl-liked") : anyCollection(raw(id));
    if (!c) throw { code: "notFound", message: "That collection does not exist.", retryable: false, action: null };
    // Long lists arrive a page at a time, as YouTube sends them.
    const long = c.trackIds.length > 2 * PAGE;
    return delay({
      id: mid(c.id), kind: kindOf(c), title: c.title, subtitle: card(c).subtitle, artists: card(c).artists, year: c.year ?? null, art: art(c.art),
      description: c.desc ?? null, stats: `${c.trackIds.length} songs`, trackCount: c.trackIds.length,
      entries: (long ? c.trackIds.slice(0, PAGE) : c.trackIds).map((t, i) => ({ entryId: c.by === "You" ? setsOf(c)[i] : `${t}#${i}`, track: track(t), setVideoId: c.by === "You" ? setsOf(c)[i] : null, videoId: track(t).id })),
      cursor: long ? `${c.id}@${PAGE}` : null, playlistId: c.id, saved: c.by === "You" ? true : false, editable: c.by === "You", privacy: c.by === "You" ? privacy.get(c.id) ?? "private" : null, related: [],
    });
  },
  async collectionMore(cursor) {
    const [cid, at] = cursor.split("@");
    const c = anyCollection(cid);
    const from = Number(at);
    if (!c || !Number.isFinite(from)) return { entries: [], cursor: null, complete: false };
    const to = Math.min(c.trackIds.length, from + PAGE);
    const entries = c.trackIds.slice(from, to).map((t, k) => ({ entryId: `${t}#${from + k}`, track: track(t), setVideoId: null, videoId: track(t).id }));
    return delay({ entries, cursor: to < c.trackIds.length ? `${c.id}@${to}` : null, complete: to >= c.trackIds.length }, 700);
  },
  async artist(id) {
    const a = M.artists.find((x) => x.id === raw(id));
    if (!a) throw { code: "notFound", message: "That artist does not exist.", retryable: false, action: null };
    return delay({
      id: mid(a.id), name: a.name, subscribers: `${a.subs} subscribers`, description: a.blurb, art: art(a.art),
      topSongs: M.tracks.filter((t) => t.artistId === a.id).slice(0, 5).map(toTrack), topSongsMore: null,
      // The release shelves link to the artist's discography, each with its own params (D35).
      shelves: [
        { ...shelf("Albums", M.collections.filter((c) => c.artistId === a.id).map(item.c)), more: { title: "Albums", browseId: `MPAD${a.id}`, params: "albums", hue: null } },
        { ...shelf("Singles & EPs", singles.filter((c) => c.artistId === a.id).map(item.c)), more: { title: "Singles & EPs", browseId: `MPAD${a.id}`, params: "singles", hue: null } },
        shelf("Fans also like", M.artists.filter((x) => x.id !== a.id).slice(0, 6).map(item.a)),
      ].filter((s) => s.items.length),
      subscribed: false, channelId: a.id, radioPlaylist: `RD${a.id}`, shufflePlaylist: null,
    });
  },
  async library(section) {
    const items: T.ShelfItem[] =
      section === "playlists" ? M.collections.filter((c) => c.kind === "playlist" || c.kind === "mix").map(item.c)
      : section === "albums" ? M.collections.filter((c) => c.kind === "album").map(item.c)
      : section === "artists" || section === "subscriptions" ? M.artists.map(item.a)
      : [...liked].map((id) => item.t(M.tracks.find((t) => mid(t.id) === id)!));
    return delay({ section, items, cursor: null });
  },
  libraryMore: async (section) => ({ section, items: [], cursor: null }),
  async lyrics(t) {
    const m = M.tracks.find((x) => mid(x.id) === t.id);
    const lines = m ? lyricLines(m) : null;
    if (lines) return { trackId: t.id, sync: "line", lines, plain: null, source: "Mock", attribution: null };
    if (m?.title.startsWith("Exit")) return { trackId: t.id, sync: "none", lines: [], plain: "Plain lyrics example\nwithout timing\nfor the fallback view", source: "Mock", attribution: null };
    return null;
  },
  playerSnapshot: async () => snap(),
  queueSnapshot: async () => ({ revision: String(qrev), entries: P.entries, index: P.index }),
  async playTracks(tracks, start, source, shuffle) {
    replace(tracks, start, source, shuffle);
  },
  async playCollection(id, start, shuffle) {
    const c = await mockApi.collection(id);
    if (!dl.online) {
      const local = c.entries.map((e) => e.track).filter((t) => isLocal(t.id));
      if (!local.length) throw { code: "network", message: "None of these songs are downloaded yet. Connect to the internet to play them.", retryable: true, action: null };
      return replace(local, shuffle ? Math.floor(Math.random() * local.length) : 0, { label: c.title, collectionId: id, radioPlaylist: null }, shuffle);
    }
    const src = { label: c.title, collectionId: id, radioPlaylist: null };
    if (shuffle) return replaceCycle(c.entries.map((e) => e.track), src);
    replace(c.entries.map((e) => e.track), start, src, false);
  },
  async playTrack(t) {
    replace([t], 0, { label: `${t.title} radio`, collectionId: null, radioPlaylist: null }, false);
  },
  async startRadio(_p, label) {
    replace(shuffled(M.tracks).slice(0, 10).map(toTrack), 0, { label, collectionId: null, radioPlaylist: null }, false);
  },
  async toggle() {
    const cur = P.index != null ? P.entries[P.index] : null;
    if (!cur) return;
    if (!P.playing && P.position >= (cur.track.duration ?? 0)) P.position = 0;
    if (!P.playing && P.phase === "error") return go(P.index!);
    P.playing = !P.playing;
    P.phase = P.playing ? "playing" : "paused";
    publish();
    anchor();
  },
  async next() {
    if (P.index == null) return;
    if (P.index < P.entries.length - 1) go(P.index + 1);
    else if (P.repeat === "all") go(0);
    else if (P.autoplay && extendRadio()) go(P.index + 1);
  },
  async prev() {
    if (P.index == null) return;
    if (P.position > 3 || P.index === 0) {
      P.position = 0;
      sgen++;
      anchor();
      publish();
    } else go(P.index - 1);
  },
  async seek(t) {
    const cur = P.index != null ? P.entries[P.index] : null;
    if (!cur) return;
    P.position = Math.min(Math.max(0, t), (cur.track.duration ?? 0) - 0.1);
    const l = mockPlayer.loop;
    if (l?.a != null && l.b != null && (P.position < l.a - 0.05 || P.position > l.b + 0.05)) endLoop();
    sgen++;
    if (P.phase === "ended") P.phase = "paused";
    publish();
    anchor();
  },
  async jumpTo(u) {
    const i = P.entries.findIndex((e) => e.uid === u);
    if (i >= 0) go(i);
  },
  async playNext(tracks) {
    const at = P.index != null ? P.index + 1 : 0;
    const es = tracks.map((t) => ({ uid: uid(), track: t, origin: "manual" as const }));
    P.entries.splice(at, 0, ...es);
    const cat = P.index != null ? P.canon.indexOf(P.entries[P.index].uid) + 1 : 0;
    P.canon.splice(cat, 0, ...es.map((e) => e.uid));
    if (P.entries.length === es.length) go(0);
    else publish(true);
  },
  async addToQueue(tracks) {
    const es = tracks.map((t) => ({ uid: uid(), track: t, origin: "manual" as const }));
    P.entries.push(...es);
    P.canon.push(...es.map((e) => e.uid));
    if (P.entries.length === es.length) go(0);
    else publish(true);
  },
  async queueRemove(u) {
    const i = P.entries.findIndex((e) => e.uid === u);
    if (i < 0 || i === P.index) return;
    P.entries.splice(i, 1);
    P.canon = P.canon.filter((x) => x !== u);
    if (P.index != null && i < P.index) P.index--;
    publish(true);
  },
  async queueMove(u, before) {
    const from = P.entries.findIndex((e) => e.uid === u);
    if (from < 0 || from === P.index) return;
    const cur = P.index != null ? P.entries[P.index].uid : null;
    const [e] = P.entries.splice(from, 1);
    const to = before ? P.entries.findIndex((x) => x.uid === before) : P.entries.length;
    P.entries.splice(to < 0 ? P.entries.length : to, 0, e);
    if (cur) P.index = P.entries.findIndex((x) => x.uid === cur);
    if (!P.shuffle) P.canon = P.entries.map((x) => x.uid);
    publish(true);
  },
  async clearUpNext() {
    if (P.index == null) return;
    const drop = new Set(P.entries.slice(P.index + 1).map((e) => e.uid));
    P.entries = P.entries.slice(0, P.index + 1);
    P.canon = P.canon.filter((x) => !drop.has(x));
    publish(true);
  },
  async setShuffle(on) {
    if (on === P.shuffle) return;
    const split = P.index != null ? P.index + 1 : 0;
    if (on) P.entries = [...P.entries.slice(0, split), ...shuffled(P.entries.slice(split))];
    else {
      const rank = new Map(P.canon.map((u, i) => [u, i]));
      P.entries = [...P.entries.slice(0, split), ...P.entries.slice(split).sort((a, b) => (rank.get(a.uid) ?? 1e9) - (rank.get(b.uid) ?? 1e9))];
    }
    P.shuffle = on;
    publish(true);
  },
  async setRepeat(m) {
    P.repeat = m;
    mockPlayer.loopRepeat = null;
    publish();
  },
  async setAutoplay(on) {
    P.autoplay = on;
    if (!on && P.index != null) {
      const keep = P.entries.filter((e, i) => i <= P.index! || e.origin !== "radio");
      if (keep.length !== P.entries.length) {
        const gone = new Set(P.entries.filter((e) => !keep.includes(e)).map((e) => e.uid));
        P.entries = keep;
        P.canon = P.canon.filter((u) => !gone.has(u));
        return publish(true);
      }
    }
    publish();
  },
  async setVolume(v) {
    P.volume = Math.min(1, Math.max(0, v));
    if (P.volume > 0) P.muted = false;
    publish();
  },
  async setMuted(m) {
    P.muted = m;
    publish();
  },
  async setLike(id, l) {
    if (l) liked.add(id);
    else liked.delete(id);
    return "done";
  },
  setSaved: async () => "done",
  setSubscribed: async () => "done",
  async playlistAdd(p, ids, dup) {
    const c = mine(p);
    if (!c) return delay("rejected" as const);
    const add = ids.map(raw);
    if (!dup && add.some((id) => c.trackIds.includes(id))) return delay("duplicate" as const);
    const sets = setsOf(c);
    for (const id of add) {
      c.trackIds.push(id);
      sets.push(`sv${++setN}`);
    }
    changed(c.id);
    return delay("done" as const);
  },
  async playlistRemove(p, entries) {
    const c = mine(p);
    if (!c) return "rejected";
    const sets = setsOf(c);
    const drop = new Set(entries.map((e) => e.setVideoId));
    const keep = c.trackIds.map((t, i) => [t, sets[i]] as const).filter(([, sv]) => !drop.has(sv));
    c.trackIds = keep.map((k) => k[0]);
    setIds.set(c.id, keep.map((k) => k[1]));
    changed(c.id);
    return "done";
  },
  async playlistCreate(title, description, pr, ids) {
    const id = `pl-new-${Date.now().toString(36)}`;
    const seed = 300 + M.collections.length;
    M.collections.push({ id, kind: "playlist", title: title.trim(), by: "You", art: { seed, h1: (seed * 47) % 360, h2: (seed * 91) % 360, motif: "orbit" }, trackIds: ids.map(raw), desc: description || undefined });
    privacy.set(id, pr);
    changed(id);
    return delay({ outcome: "done" as const, id: mid(id), playlistId: id }, 300);
  },
  async playlistEdit(p, e) {
    const c = mine(p);
    if (!c) return "rejected";
    if (e.title != null) c.title = e.title.trim();
    if (e.description != null) c.desc = e.description || undefined;
    if (e.privacy) privacy.set(c.id, e.privacy);
    changed(c.id);
    return delay("done" as const, 250);
  },
  async playlistDelete(p) {
    const c = mine(p);
    if (!c) return "rejected";
    M.collections.splice(M.collections.indexOf(c), 1);
    changed(c.id);
    return delay("done" as const, 250);
  },
  async playlistMove(p, sv, before) {
    const c = mine(p);
    if (!c) return "rejected";
    const sets = setsOf(c);
    const from = sets.indexOf(sv);
    if (from < 0) return "rejected";
    const [t] = c.trackIds.splice(from, 1);
    const [s1] = sets.splice(from, 1);
    const to = before ? sets.indexOf(before) : sets.length;
    c.trackIds.splice(to < 0 ? sets.length : to, 0, t);
    sets.splice(to < 0 ? sets.length : to, 0, s1);
    return delay("done" as const, 150);
  },
  async history() {
    if (auth.state !== "signedIn") throw { code: "authRequired", message: "Sign in to YouTube Music to use this.", retryable: false, action: "signIn" };
    return delay({ groups: hist.filter((g) => g.rows.length), notice: null, paused: false, cursor: null });
  },
  historyMore: async () => ({ groups: [], notice: null, paused: false, cursor: null }),
  async historyRemove(ids) {
    for (const g of hist) g.rows = g.rows.filter((r) => !ids.includes(r.rowId));
    return delay("done" as const, 200);
  },
  counterpart: (t) => delay(counterpartOf(t), 150),
  async switchVersion(u) {
    const i = P.entries.findIndex((e) => e.uid === u);
    const e = P.entries[i];
    const c = e && counterpartOf(e.track);
    if (!c) throw { code: "notFound", message: "YouTube Music has no other version of this.", retryable: false, action: null };
    const seg = c.segments.find((s) => P.position >= (s.fromStart ?? 0) && P.position < (s.fromStart ?? 0) + (s.duration ?? 0));
    const at = i === P.index && seg ? (seg.toStart ?? 0) + (P.position - (seg.fromStart ?? 0)) : 0;
    P.entries[i] = { ...e, track: c.track };
    if (i !== P.index) return publish(true);
    const was = P.playing;
    go(i, at);
    if (!was) {
      P.playing = false;
      P.phase = "paused";
      publish();
      anchor();
    }
  },
  async playVersion(t) {
    const c = counterpartOf(t);
    if (!c) throw { code: "notFound", message: "YouTube Music has no other version of this.", retryable: false, action: null };
    await mockApi.playNext([c.track]);
    if (P.index != null && P.entries.length > 1) go(P.index + 1);
  },
  async setNormalise(on) {
    normalise = on;
  },
  async setReportHistory(on) {
    reportHistory = on;
  },
  authState: async () => auth,
  browserProfiles: async () => [{ id: "edge:Default", browser: "Microsoft Edge", name: "Profile 1" }],
  async importBrowser() {
    auth = { state: "choosingChannel", channels, source: "edge:Default" };
    h?.auth(auth);
    return delay(auth, 400);
  },
  async selectChannel(key) {
    const c = channels.find((x) => x.key === key) ?? channels[0];
    auth = { state: "signedIn", channel: c, channels, source: "edge:Default", premium: null };
    h?.auth(auth);
    return auth;
  },
  reloadChannels: async () => channels,
  async signOut() {
    auth = { state: "anonymous" };
    h?.auth(auth);
  },
  async beginSignin() {
    h?.signin({ stage: "opened", message: null });
    setTimeout(() => mockApi.importBrowser("webview"), 800);
  },
  cancelSignin: async () => {},
  downloadsSnapshot: async () => structuredClone({ summary: dlSummary(), items: [...dl.items.values()], collections: dlKept() }),
  async downloadTracks(tracks) {
    let n = 0;
    for (const t of tracks) {
      dl.pins.add(t.id);
      const it = dl.items.get(t.id);
      if (!it || it.state === "failed") {
        dlSet(t.id, "queued");
        n++;
      }
    }
    dlEmit();
    return n;
  },
  async downloadRemove(ids) {
    const kept: T.DownloadRemoved["kept"] = [];
    let removed = 0;
    for (const id of ids) {
      dl.pins.delete(id);
      const k = keptBy(id);
      if (k) kept.push({ trackId: id, collection: k.card.title as string | null });
      else if (dl.items.has(id)) {
        if (dl.active?.id === id) dl.active = null;
        dlSet(id, "removed");
        removed++;
      }
    }
    dlEmit();
    return { removed, kept };
  },
  async downloadKeep(id) {
    const c = await mockApi.collection(id);
    const ids = c.entries.map((e) => e.track.id);
    dl.kept.set(id, { card: { id, kind: c.kind, title: c.title, subtitle: c.subtitle, artists: c.artists, year: c.year, art: c.art, playId: null }, ids, paused: false, synced: now() });
    for (const t of ids) if (!dl.items.has(t)) dlSet(t, "queued");
    dlChanged();
  },
  async downloadUnkeep(id) {
    const k = dl.kept.get(id);
    if (!k) throw { code: "notFound", message: "That isn't kept downloaded.", retryable: false, action: null };
    dl.kept.delete(id);
    let removed = 0;
    for (const t of new Set(k.ids)) {
      if (dl.pins.has(t) || keptBy(t) || !dl.items.has(t)) continue;
      if (dl.active?.id === t) dl.active = null;
      dlSet(t, "removed");
      removed++;
    }
    dlChanged();
    return { removed, kept: [] };
  },
  async downloadsSync() {
    for (const k of dl.kept.values()) k.synced = now();
    dlChanged();
  },
  async downloadsPause(p) {
    dl.paused = p;
    dlEmit();
  },
  async downloadPauseCollection(id, p) {
    const k = dl.kept.get(id);
    if (k) k.paused = p;
    dlChanged();
  },
  async downloadsRetry() {
    for (const i of [...dl.items.values()]) if (i.state === "failed") dlSet(i.track.id, "queued");
    dlEmit();
  },
  async downloadsRemoveAll() {
    dl.pins.clear();
    dl.kept.clear();
    dl.active = null;
    for (const id of [...dl.items.keys()]) dlSet(id, "removed");
    dlChanged();
  },
  downloadsStorage: async () => {
    const s = dlSummary();
    return { songs: s.songs, bytes: s.bytes, location: "~/Library/Application Support/app.encore.desktop/downloads" };
  },
  connectivityCheck: async () => dl.online,
  updateStatus: async () => upStatus(),
  updateCheck: () => upCheck(true),
  async updateDownload() {
    upDownload(true);
    return upStatus();
  },
  // Like a restart into the new version, without reloading the page.
  async updateInstall() {
    if (up.phase !== "ready" || !up.available) throw { code: "invalid", message: "No update is ready to install. Check for updates first.", retryable: false, action: null };
    up.phase = "installing";
    upEmit();
    await new Promise((r) => setTimeout(r, 1500));
    up.updated = up.available;
    up.current = up.available.version;
    up.available = null;
    up.phase = "upToDate";
    upEmit();
  },
  async updateSkip(v) {
    up.skipped = v;
    if (up.available?.version === v && (up.phase === "downloading" || up.phase === "ready")) up.phase = "available";
    upEmit();
    return upStatus();
  },
  async updateUnskip() {
    up.skipped = null;
    if (up.auto && up.phase === "available") upDownload(false);
    upEmit();
    return upStatus();
  },
  async setAutoUpdate(on) {
    up.auto = on;
    if (!on && up.phase === "downloading" && !up.requested) up.phase = "available";
    if (on && up.phase === "available" && up.skipped !== up.available?.version) upDownload(false);
    upEmit();
    return upStatus();
  },
  artUrl: () => "",
  features: mockFeatures,
};

/** What the feature mocks (./mock/<area>.ts) share with this one: the
 *  simulated player and its events, the catalogue and the handlers. Read
 *  it inside functions only (the feature mocks load before this file has
 *  finished). */
export const mockKit = {
  get handlers() {
    return h;
  },
  /** The simulated player's state; `publish()` sends it. */
  player: P,
  publish,
  snapshot: snap,
  track,
  delay,
  /** Online, as the mock's connection says (`?offline`). */
  get online() {
    return dl.online;
  },
  get signedIn() {
    return auth.state === "signedIn";
  },
};
