// Mock backend for `bun run dev` in a plain browser: the invented catalogue
// from mockdata.ts and a simulated player clock, behind the same API as Tauri.
import * as M from "./mockdata";
import type { Api, Handlers } from "./index";
import type * as T from "./types";

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
    subtitle: c.year ? `${c.kind === "album" ? "Album" : "Playlist"} • ${c.by} • ${c.year}` : `${c.kind === "mix" ? "Mix" : "Playlist"} • ${c.by}`,
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

const liked = new Set<string>(M.collections.find((c) => c.id === "pl-liked")!.trackIds.map(mid));
const delay = <T>(v: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(structuredClone(v)), ms));

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
function played(t: T.Track) {
  if (!reportHistory || auth.state !== "signedIn") return;
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
    error: null, skippedInARow: 0, queueLoading: false, noOutput: false,
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
  P.index = i;
  P.position = at;
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
  if (P.repeat === "one") {
    P.position = 0;
    sgen++;
    anchor();
  } else if (P.index < P.entries.length - 1) go(P.index + 1);
  else if (P.repeat === "all") go(0);
  else if (P.autoplay && extendRadio()) go(P.index + 1);
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
  if (!P.playing || !cur) return;
  P.position += dt;
  if (P.position >= (cur.track.duration ?? 0)) ended();
}, 100);
setInterval(() => P.playing && anchor(), 500);

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
      notice: null,
      auth,
      player: snap(),
      queue: { revision: String(qrev), entries: P.entries, index: P.index },
      resolver: { ready: true, python: "mock", jsRuntime: "mock", ytDlp: "mock", message: null },
    }, 10);
  },
  resolverSetup: async () => ({ ready: true, python: "mock", jsRuntime: "mock", ytDlp: "mock", message: null }),
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
        shelf("New albums & singles", M.collections.filter((c) => c.kind === "album").map(item.c)),
        shelf("Moods & genres", M.moods.map((m) => ({ type: "link" as const, title: m.name, browseId: "FEmusic_moods_and_genres_category", params: m.name, hue: m.h }))),
      ],
    }),
  browsePage: (b, p) =>
    delay({
      title: p ?? (b === "FEmusic_charts" ? "Charts" : b === "FEmusic_new_releases" ? "New releases" : "Moods & genres"),
      chips: [],
      cursor: null,
      shelves: [shelf("Playlists", M.collections.filter((c) => c.kind !== "album").map(item.c)), shelf("Albums", M.collections.filter((c) => c.kind === "album").map(item.c))],
    }),
  moreShelves: async () => ({ title: null, chips: [], cursor: null, shelves: [] }),
  async search(q) {
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
    const c = M.collections.find((x) => x.id === raw(id));
    if (!c) throw { code: "notFound", message: "That collection does not exist.", retryable: false, action: null };
    return delay({
      id: mid(c.id), kind: kindOf(c), title: c.title, subtitle: card(c).subtitle, artists: card(c).artists, year: c.year ?? null, art: art(c.art),
      description: c.desc ?? null, stats: `${c.trackIds.length} songs`, trackCount: c.trackIds.length,
      entries: c.trackIds.map((t, i) => ({ entryId: c.by === "You" ? setsOf(c)[i] : `${t}#${i}`, track: track(t), setVideoId: c.by === "You" ? setsOf(c)[i] : null, videoId: track(t).id })),
      cursor: null, playlistId: c.id, saved: c.by === "You" ? true : false, editable: c.by === "You", privacy: c.by === "You" ? privacy.get(c.id) ?? "private" : null, related: [],
    });
  },
  collectionMore: async () => ({ entries: [], cursor: null }),
  async artist(id) {
    const a = M.artists.find((x) => x.id === raw(id));
    if (!a) throw { code: "notFound", message: "That artist does not exist.", retryable: false, action: null };
    return delay({
      id: mid(a.id), name: a.name, subscribers: `${a.subs} subscribers`, description: a.blurb, art: art(a.art),
      topSongs: M.tracks.filter((t) => t.artistId === a.id).slice(0, 5).map(toTrack), topSongsMore: null,
      shelves: [shelf("Albums", M.collections.filter((c) => c.artistId === a.id).map(item.c)), shelf("Fans also like", M.artists.filter((x) => x.id !== a.id).slice(0, 6).map(item.a))].filter((s) => s.items.length),
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
    const s = shuffle ? Math.floor(Math.random() * c.entries.length) : start;
    replace(c.entries.map((e) => e.track), s, { label: c.title, collectionId: id, radioPlaylist: null }, shuffle);
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
    publish();
  },
  async setAutoplay(on) {
    P.autoplay = on;
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
  artUrl: () => "",
};
