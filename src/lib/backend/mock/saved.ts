// The mock of saved (lane C) for `bun run dev`: bookmarks, saved queues and
// folders, kept per owner (signed in or not) like the real store. It starts
// with a few of each; `?nosaved` starts with none. Shared mock state (the
// simulated player, the catalogue) is `mockKit` in ../mock.ts.
import type { SavedApi } from "../features/saved";
import type * as T from "../types";
import { mockApi, mockKit } from "../mock";

type Box = { bookmarks: T.Bookmark[]; queues: (T.SavedQueueInfo & { entries: T.QueueEntry[]; index: number | null; shuffle: boolean; repeat: T.Repeat; source: T.QueueSource | null })[]; folders: T.Folder[] };
const boxes = new Map<string, Promise<Box>>();
let n = 0;
const nid = () => `s${++n}`;
const now = () => String(Math.floor(Date.now() / 1000));
const fail = (message: string, code = "invalid") => ({ code, message, retryable: false, action: null });
const clean = (s: string) => s.replace(/[\u0000-\u001f]/g, "").trim().slice(0, 120);

async function card(id: string): Promise<T.CollectionCard> {
  const c = await mockApi.collection(`mock:${id}`);
  return { id: c.id, kind: c.kind, title: c.title, subtitle: c.subtitle, artists: c.artists, year: c.year, art: c.art, playId: null };
}

async function seed(b: Box) {
  if (new URLSearchParams(location.search).has("nosaved")) return;
  const t = mockKit.track;
  const day = 86400;
  const at = (ago: number) => String(Math.floor(Date.now() / 1000) - ago);
  const song = (id: string) => ({ ...t(id), liked: null });
  const night = await card("al-night");
  b.bookmarks = [
    { id: nid(), kind: "moment", target: { type: "track", value: song("al-glass-1") }, position: 102, title: t("al-glass-1").title, subtitle: t("al-glass-1").artists[0].name, art: t("al-glass-1").art, created: at(3600) },
    { id: nid(), kind: "page", target: { type: "page", value: { name: "collection", id: night.id, params: null } }, position: null, title: night.title, subtitle: "Album · Night Office", art: night.art, created: at(day) },
    { id: nid(), kind: "track", target: { type: "track", value: song("al-yakou-2") }, position: null, title: t("al-yakou-2").title, subtitle: t("al-yakou-2").artists[0].name, art: t("al-yakou-2").art, created: at(2 * day) },
    { id: nid(), kind: "page", target: { type: "page", value: { name: "artist", id: "mock:lumen" } }, position: null, title: "Lumen Drive", subtitle: "Artist", art: t("al-daybreak-1").art, created: at(4 * day) },
    { id: nid(), kind: "moment", target: { type: "track", value: song("al-velvet-1") }, position: 61, title: t("al-velvet-1").title, subtitle: t("al-velvet-1").artists[0].name, art: t("al-velvet-1").art, created: at(6 * day) },
  ];
  const tracks = ["al-tide-1", "al-tide-2", "al-night-1", "al-night-2", "al-hymnal-3", "al-tide-3", "al-night-3"].map(t);
  const entries = tracks.map((track) => ({ uid: nid(), track, origin: "manual" as const }));
  b.queues = [
    {
      id: nid(), name: "Sunday morning", songs: entries.length, duration: tracks.reduce((s, x) => s + (x.duration ?? 0), 0),
      current: tracks[2], position: 74, updated: at(2 * day), entries, index: 2, shuffle: false, repeat: "off", source: { label: "Sunday morning", collectionId: null, radioPlaylist: null },
    },
  ];
  b.folders = [
    { id: nid(), name: "After dark", cover: null, collapsed: false, items: await Promise.all(["pl-late", "mx-2", "al-night"].map(async (id) => { const c = await card(id); return { itemId: c.id, card: c }; })) },
    { id: nid(), name: "Rainy days", cover: "mock:9:wave", collapsed: true, items: await Promise.all(["pl-rain", "al-glass"].map(async (id) => { const c = await card(id); return { itemId: c.id, card: c }; })) },
  ];
}

function box(): Promise<Box> {
  const k = mockKit.signedIn ? "you" : "anonymous";
  let b = boxes.get(k);
  if (!b) {
    const fresh: Box = { bookmarks: [], queues: [], folders: [] };
    b = seed(fresh).then(() => fresh);
    boxes.set(k, b);
  }
  return b;
}

const same = (a: T.BookmarkTarget, b: T.BookmarkTarget) => (a.type === "track" && b.type === "track" ? a.value.id === b.value.id : JSON.stringify(a.value) === JSON.stringify(b.value));
const info = ({ entries: _e, index: _i, shuffle: _s, repeat: _r, source: _so, ...q }: Box["queues"][number]): T.SavedQueueInfo => q;
const folder = async (id: string) => {
  const f = (await box()).folders.find((x) => x.id === id);
  if (!f) throw fail("That folder is gone.", "notFound");
  return f;
};
const ok = <X>(v: X) => new Promise<X>((r) => setTimeout(() => r(structuredClone(v)), 60));

export const mockSaved: SavedApi = {
  bookmarks: async () => ok((await box()).bookmarks),
  async bookmark(target, position, title, subtitle, art) {
    const b = await box();
    const kind: T.BookmarkKind = target.type === "page" ? "page" : position == null ? "track" : "moment";
    const found = b.bookmarks.find((x) => x.kind === kind && same(x.target, target) && (x.position == null || position == null || Math.abs(x.position - position) < 2));
    if (found) return ok(found);
    const v: T.Bookmark = { id: nid(), kind, target: target.type === "track" ? { type: "track", value: { ...target.value, liked: null } } : target, position, title, subtitle, art, created: now() };
    b.bookmarks.unshift(v);
    return ok(v);
  },
  async unbookmark(id) {
    const b = await box();
    b.bookmarks = b.bookmarks.filter((x) => x.id !== id);
  },

  queues: async () => ok((await box()).queues.map(info)),
  async saveQueue(name) {
    const b = await box();
    const P = mockKit.player;
    const nm = clean(name);
    if (!nm) throw fail("Give it a name.");
    if (!P.entries.length) throw fail("The queue is empty.");
    const old = b.queues.find((q) => q.name === nm);
    const durs = P.entries.map((e) => e.track.duration);
    const q = {
      id: old?.id ?? nid(), name: nm, songs: P.entries.length, duration: durs.every((d) => d != null) ? durs.reduce((s, d) => s! + d!, 0) : null,
      current: P.index != null ? P.entries[P.index].track : null, position: P.position, updated: now(),
      entries: structuredClone(P.entries), index: P.index, shuffle: P.shuffle, repeat: P.repeat, source: P.source,
    };
    b.queues = [q, ...b.queues.filter((x) => x.id !== q.id)];
    return ok(info(q));
  },
  async restoreQueue(id) {
    const q = (await box()).queues.find((x) => x.id === id);
    if (!q) throw fail("That saved queue is gone.", "notFound");
    const P = mockKit.player;
    P.entries = structuredClone(q.entries);
    P.canon = P.entries.map((e) => e.uid);
    P.index = q.index;
    P.position = q.position ?? 0;
    P.shuffle = q.shuffle;
    P.repeat = q.repeat;
    P.source = q.source;
    P.playing = false;
    P.phase = q.index != null ? "paused" : "idle";
    P.error = null;
    mockKit.publish(true);
  },
  async removeQueue(id) {
    const b = await box();
    b.queues = b.queues.filter((x) => x.id !== id);
  },

  folders: async () => ok((await box()).folders),
  async createFolder(name, items) {
    const b = await box();
    const nm = clean(name);
    if (!nm) throw fail("Give it a name.");
    const ids = new Set(items.map((c) => c.id));
    for (const f of b.folders) f.items = f.items.filter((i) => !ids.has(i.itemId));
    const f: T.Folder = { id: nid(), name: nm, cover: null, collapsed: false, items: items.map((c) => ({ itemId: c.id, card: c })) };
    b.folders.push(f);
    return ok(f);
  },
  async renameFolder(id, name) {
    const nm = clean(name);
    if (!nm) throw fail("Give it a name.");
    (await folder(id)).name = nm;
  },
  async removeFolder(id) {
    const b = await box();
    b.folders = b.folders.filter((f) => f.id !== id);
  },
  async setCollapsed(id, collapsed) {
    (await folder(id)).collapsed = collapsed;
  },
  async file(id, c) {
    const b = await box();
    const f = await folder(id);
    for (const o of b.folders) if (o !== f) o.items = o.items.filter((i) => i.itemId !== c.id);
    if (!f.items.some((i) => i.itemId === c.id)) f.items.push({ itemId: c.id, card: c });
  },
  async unfile(id, item) {
    const f = await folder(id);
    f.items = f.items.filter((i) => i.itemId !== item);
  },
  async moveItem(id, item, before) {
    const f = await folder(id);
    const it = f.items.find((i) => i.itemId === item);
    if (!it) return;
    const rest = f.items.filter((i) => i !== it);
    const at = before ? rest.findIndex((i) => i.itemId === before) : -1;
    rest.splice(at < 0 ? rest.length : at, 0, it);
    f.items = rest;
  },
  // No file picker in the mock: a drawn cover stands in for the picture.
  async chooseCover(id) {
    const f = await folder(id);
    const motifs = ["orbit", "stripes", "grid", "wave", "sun", "blocks"];
    f.cover = `mock:${300 + Math.floor(Math.random() * 600)}:${motifs[Math.floor(Math.random() * motifs.length)]}`;
    return ok(true);
  },
  async clearCover(id) {
    (await folder(id)).cover = null;
  },
  // The file itself, as a download (the app shows a save sheet).
  async exportFolder(id) {
    const f = await folder(id);
    const file = { format: "encorefolder", version: 1, name: f.name, items: f.items.map((i) => ({ link: `https://music.youtube.com/playlist?list=${i.itemId.replace(/^mock:/, "")}`, title: i.card.title, subtitle: i.card.subtitle, kind: i.card.kind })) };
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([JSON.stringify(file, null, 2)], { type: "application/json" }));
    a.download = `${f.name}.encorefolder`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
    return true;
  },
  // No file picker in the mock: a folder someone "sent".
  async importFolder() {
    const b = await box();
    const mine = new Set(b.folders.flatMap((f) => f.items.map((i) => i.itemId)));
    const cards = (await Promise.all(["al-sora", "al-juniper", "pl-jp", "mx-3"].map(card))).filter((c) => !mine.has(c.id));
    if (!cards.length) throw fail("Everything in this folder is already in your folders.");
    return mockSaved.createFolder("From Aki: road trip", cards);
  },
};
