// Saved (lane C): bookmarks (D25), saved queues (D33), folders and their covers (D34).
//
// This module is the area's UI state and actions (`call((a) => a.features.saved.…)`). It registers
// its menu items, palette commands and the B key with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { call, type ArtRef, type BackendError, type Bookmark, type BookmarkTarget, type CollectionCard, type Folder, type PageRef, type SavedQueueInfo, type Track } from "$lib/backend";
import { catalog } from "$lib/catalog.svelte";
import { fmt, kindLabel } from "$lib/data";
import { account, downloads, nav, newPlaylist, player, ui, type MenuItem, type View } from "$lib/state.svelte";
import { addKey, artistMenuItems, collectionMenuItems, deckMenuItems, paletteItems, registerFeature, trackMenuItems, type PaletteItem } from "./hooks";

/** The name sheet (overlays/Saved.svelte): a folder's or a saved queue's name. */
export interface NameSheet {
  title: string;
  lead?: string;
  value: string;
  confirm: string;
  /** A note while the name is typed ("replaces …"), or null. */
  note?: (name: string) => string | null;
  run: (name: string) => Promise<void>;
}

const songsText = (n: number) => (n === 1 ? "1 song" : `${n} songs`);
const listsText = (n: number) => (n === 1 ? "1 playlist or album" : `${n} playlists and albums`);
const err = (e: unknown) => (e as BackendError)?.message ?? String(e);

/** "1 h 12 min", "43 min", "less than a minute". */
export function spanText(s: number) {
  const m = Math.round(s / 60);
  if (m < 1) return "less than a minute";
  const h = Math.floor(m / 60);
  return h ? `${h} h ${m % 60} min` : `${m} min`;
}

class Saved {
  bookmarks = $state<Bookmark[]>([]);
  queues = $state<SavedQueueInfo[]>([]);
  folders = $state<Folder[]>([]);
  naming = $state<NameSheet | null>(null);
  /** The lists for the current account have arrived. */
  loaded = $state(false);
  #seq = 0;

  /** Every list for the account now (again after a switch). */
  async load() {
    const my = ++this.#seq;
    const [b, q, f] = await Promise.all([
      call((a) => a.features.saved.bookmarks()).catch((e) => (console.error("bookmarks:", e), null)),
      call((a) => a.features.saved.queues()).catch(() => null),
      call((a) => a.features.saved.folders()).catch(() => null),
    ]);
    if (my !== this.#seq) return;
    this.loaded = true;
    if (b) this.bookmarks = b;
    if (q) this.queues = q;
    if (f) {
      this.folders = f;
      for (const x of f) catalog.addItems(x.items.map((i) => ({ type: "collection" as const, ...i.card })));
    }
  }

  async reloadFolders() {
    const f = await call((a) => a.features.saved.folders()).catch(() => null);
    if (f) this.folders = f;
  }

  /** The folder a playlist or album is filed in. */
  folderOf(id: string): Folder | undefined {
    return this.folders.find((f) => f.items.some((i) => i.itemId === id));
  }

  /** Every list filed in a folder (the sidebar leaves them out of the loose list). */
  get filed(): Set<string> {
    return new Set(this.folders.flatMap((f) => f.items.map((i) => i.itemId)));
  }
}

export const saved = new Saved();

// ---------- bookmarks (D25) ----------

/** The page a view shows, if it can be bookmarked. */
export function pageRefOf(v: View): PageRef | null {
  if (v.name === "collection") return { name: "collection", id: v.id, params: v.params ?? null };
  if (v.name === "artist") return { name: "artist", id: v.id };
  if (v.name === "page") return { name: "page", browseId: v.browseId, params: v.params ?? null, title: v.title ?? null };
  return null;
}

function viewOf(r: PageRef): View {
  if (r.name === "collection") return { name: "collection", id: r.id, params: r.params };
  if (r.name === "artist") return { name: "artist", id: r.id };
  return { name: "page", browseId: r.browseId, params: r.params, title: r.title ?? undefined };
}

const sameRef = (a: PageRef, b: PageRef) => JSON.stringify(viewOf(a)) === JSON.stringify(viewOf(b));

function pageBookmark(r: PageRef): Bookmark | undefined {
  return saved.bookmarks.find((b) => b.kind === "page" && b.target.type === "page" && sameRef(b.target.value, r));
}

function trackBookmark(id: string): Bookmark | undefined {
  return saved.bookmarks.find((b) => b.kind === "track" && b.target.type === "track" && b.target.value.id === id);
}

/** What a page bookmark shows: the title, a line under it, the cover. */
function pageLabel(r: PageRef): { title: string; subtitle: string; art: ArtRef | null } {
  if (r.name === "collection") {
    const c = catalog.collection(r.id);
    const sub = c ? `${kindLabel[c.kind]}${c.artists[0] ? ` · ${c.artists.map((a) => a.name).join(", ")}` : ""}` : "Playlist";
    return { title: c?.title ?? "Playlist", subtitle: r.params ? `${sub} · all songs` : sub, art: (c?.art as ArtRef | null) ?? null };
  }
  if (r.name === "artist") {
    const a = catalog.artist(r.id);
    return { title: a?.name ?? "Artist", subtitle: "Artist", art: (a?.art as ArtRef | null) ?? null };
  }
  return { title: r.title ?? "Page", subtitle: "Page", art: null };
}

async function add(target: BookmarkTarget, position: number | null, title: string, subtitle: string, art: ArtRef | null, said: string) {
  try {
    const b = await call((a) => a.features.saved.bookmark(target, position, title, subtitle, art));
    saved.bookmarks = [b, ...saved.bookmarks.filter((x) => x.id !== b.id)];
    ui.say(said);
  } catch (e) {
    ui.say(`Could not bookmark: ${err(e)}`);
  }
}

export async function removeBookmark(b: Bookmark, quiet = false) {
  const before = saved.bookmarks;
  saved.bookmarks = before.filter((x) => x.id !== b.id);
  try {
    await call((a) => a.features.saved.unbookmark(b.id));
    if (!quiet) ui.say("Removed the bookmark");
  } catch (e) {
    saved.bookmarks = before;
    ui.say(`Could not remove the bookmark: ${err(e)}`);
  }
}

export function togglePageBookmark(r: PageRef) {
  const had = pageBookmark(r);
  if (had) return void removeBookmark(had);
  const l = pageLabel(r);
  void add({ type: "page", value: r }, null, l.title, l.subtitle, l.art, `Bookmarked “${l.title}”`);
}

export function toggleTrackBookmark(t: Track) {
  const had = trackBookmark(t.id);
  if (had) return void removeBookmark(had);
  void add({ type: "track", value: t }, null, t.title, t.artists.map((a) => a.name).join(", "), t.art, `Bookmarked “${t.title}”`);
}

/** The song that's playing, at this moment (B). */
export function bookmarkMoment() {
  const t = player.current;
  if (!t) return ui.say("Play a song to bookmark a moment in it");
  const at = Math.floor(player.position);
  void add({ type: "track", value: t }, at, t.title, t.artists.map((a) => a.name).join(", "), t.art, `Bookmarked ${fmt(at)} of “${t.title}”`);
}

/** Play a song from `at` seconds: the playing one seeks; another starts
 *  (with its radio, as a song does anywhere else) and seeks once it plays. */
function playAt(t: Track, at: number) {
  if (!downloads.playable(t)) return ui.say(`“${t.title}” isn't downloaded. Connect to the internet to play it.`);
  if (player.current?.id === t.id) {
    player.seek(at);
    if (!player.playing) player.toggle();
    return;
  }
  catalog.addTracks([t]);
  if (ui.online) player.playTrack(t.id);
  else player.playTracks([t], 0, { label: "Bookmarks", collectionId: null, radioPlaylist: null });
  const until = Date.now() + 20_000;
  const wait = () => {
    if (player.current?.id === t.id && player.phase === "playing") return player.seek(at);
    if (Date.now() < until && player.phase !== "error") setTimeout(wait, 150);
  };
  setTimeout(wait, 150);
}

export function openBookmark(b: Bookmark) {
  if (b.target.type === "page") return nav.go(viewOf(b.target.value));
  const t = b.target.value;
  if (b.kind === "moment" && b.position != null) return playAt(t, b.position);
  catalog.addTracks([t]);
  if (!downloads.playable(t)) return ui.say(`“${t.title}” isn't downloaded. Connect to the internet to play it.`);
  if (ui.online) player.playTrack(t.id);
  else player.playTracks([t], 0, { label: "Bookmarks", collectionId: null, radioPlaylist: null });
}

/** A bookmark's line under its title. */
export function bookmarkSub(b: Bookmark) {
  if (b.kind === "moment" && b.position != null) return `From ${fmt(b.position)} · ${b.subtitle}`;
  return b.subtitle;
}

// ---------- saved queues (D33) ----------

export function saveQueue() {
  if (!player.queue.length) return ui.say("The queue is empty");
  saved.naming = {
    title: "Save queue",
    lead: `${songsText(player.queue.length)}${player.current ? `, paused at “${player.current.title}” when you restore it` : ""}.`,
    value: player.source?.label ?? "My queue",
    confirm: "Save",
    note: (n) => (saved.queues.some((q) => q.name === n.trim()) ? `Replaces the saved queue “${n.trim()}”.` : null),
    run: async (name) => {
      const q = await call((a) => a.features.saved.saveQueue(name));
      saved.queues = [q, ...saved.queues.filter((x) => x.id !== q.id)];
      ui.say(`Saved the queue as “${q.name}”`);
    },
  };
}

export async function restoreQueue(q: SavedQueueInfo) {
  try {
    await call((a) => a.features.saved.restoreQueue(q.id));
    ui.say(q.current ? `Restored “${q.name}”, paused at “${q.current.title}”` : `Restored “${q.name}”`);
  } catch (e) {
    ui.say(`Could not restore: ${err(e)}`);
  }
}

export async function removeQueue(q: SavedQueueInfo) {
  const before = saved.queues;
  saved.queues = before.filter((x) => x.id !== q.id);
  try {
    await call((a) => a.features.saved.removeQueue(q.id));
    ui.say(`Deleted the saved queue “${q.name}”`);
  } catch (e) {
    saved.queues = before;
    ui.say(`Could not delete: ${err(e)}`);
  }
}

/** The queue panel's ⋯ menu. */
export function queueMenu(): MenuItem[] {
  const ids = player.queue.map((e) => e.track.id);
  const items: MenuItem[] = [
    { label: "Save queue…", icon: "bookmark", disabled: ids.length ? undefined : "The queue is empty.", run: saveQueue },
  ];
  if (account.signedIn) items.push({ label: "Save as playlist…", icon: "plus", disabled: !ids.length ? "The queue is empty." : ui.online ? undefined : "You're offline. Connect to the internet to create a playlist.", run: () => newPlaylist(ids) });
  for (const q of saved.queues.slice(0, 8)) {
    items.push({
      label: `${q.name} · ${songsText(q.songs)}`,
      icon: "queue",
      run: () =>
        ui.openMenuAgain([
          { label: q.current ? `Restore, paused at “${q.current.title}”` : "Restore", icon: "play", run: () => void restoreQueue(q) },
          { label: "Delete saved queue", icon: "trash", danger: true, run: () => void removeQueue(q) },
        ]),
    });
  }
  if (player.upNext.length) items.push({ label: "Clear upcoming", icon: "x", danger: true, run: () => (player.clearUpNext(), ui.say("Cleared upcoming songs")) });
  return items;
}

// ---------- folders (D34) ----------

/** The card a folder keeps for a playlist or album (shown offline). */
function cardOf(id: string): CollectionCard | null {
  const c = catalog.collection(id);
  if (!c) return null;
  return { id: c.id, kind: c.kind, title: c.title, subtitle: c.subtitle, artists: c.artists, year: c.year, art: c.art, playId: "playId" in c ? c.playId : null };
}

export function newFolder(withItem?: string) {
  const card = withItem ? cardOf(withItem) : null;
  saved.naming = {
    title: "New folder",
    lead: card ? `Starts with “${card.title}”.` : "Drag playlists onto it in the sidebar, or choose Add to folder in a playlist's or album's menu.",
    value: "",
    confirm: "Create",
    run: async (name) => {
      const f = await call((a) => a.features.saved.createFolder(name, card ? [card] : []));
      await saved.reloadFolders();
      ui.say(card ? `Added “${card.title}” to “${f.name}”` : `Created “${f.name}”`);
    },
  };
}

export function renameFolder(f: Folder) {
  saved.naming = {
    title: "Rename folder",
    value: f.name,
    confirm: "Rename",
    run: async (name) => {
      await call((a) => a.features.saved.renameFolder(f.id, name));
      await saved.reloadFolders();
    },
  };
}

export function deleteFolder(f: Folder) {
  ui.confirm = {
    title: "Delete this folder?",
    text: f.items.length ? `“${f.name}” is removed from the sidebar. The ${listsText(f.items.length)} in it stay in your library.` : `“${f.name}” is removed from the sidebar.`,
    confirm: "Delete folder",
    danger: true,
    run: async () => {
      try {
        await call((a) => a.features.saved.removeFolder(f.id));
        await saved.reloadFolders();
        ui.say(`Deleted “${f.name}”`);
        if (nav.view.name === "folder" && nav.view.id === f.id) nav.go({ name: "library" });
      } catch (e) {
        ui.say(`Could not delete: ${err(e)}`);
      }
    },
  };
}

/** File a playlist or album in a folder (out of any other one). */
export async function fileIn(f: Folder, id: string, quiet = false) {
  const card = cardOf(id);
  if (!card) return;
  if (f.items.some((i) => i.itemId === id)) return;
  try {
    await call((a) => a.features.saved.file(f.id, card));
    await saved.reloadFolders();
    if (!quiet) ui.say(`Moved “${card.title}” to “${f.name}”`);
  } catch (e) {
    ui.say(`Could not add it to the folder: ${err(e)}`);
  }
}

export async function unfile(f: Folder, id: string) {
  try {
    await call((a) => a.features.saved.unfile(f.id, id));
    await saved.reloadFolders();
    ui.say(`Removed “${catalog.collection(id)?.title ?? "it"}” from “${f.name}”`);
  } catch (e) {
    ui.say(`Could not remove it: ${err(e)}`);
  }
}

export async function moveInFolder(f: Folder, id: string, before: string | null) {
  try {
    await call((a) => a.features.saved.moveItem(f.id, id, before));
    await saved.reloadFolders();
  } catch (e) {
    ui.say(`Could not move it: ${err(e)}`);
  }
}

export async function setCollapsed(f: Folder, collapsed: boolean) {
  saved.folders = saved.folders.map((x) => (x.id === f.id ? { ...x, collapsed } : x));
  await call((a) => a.features.saved.setCollapsed(f.id, collapsed)).catch(() => {});
}

export async function chooseCover(f: Folder) {
  try {
    if (await call((a) => a.features.saved.chooseCover(f.id))) {
      await saved.reloadFolders();
      ui.say("Cover changed");
    }
  } catch (e) {
    ui.say(err(e), "error");
  }
}

async function clearCover(f: Folder) {
  try {
    await call((a) => a.features.saved.clearCover(f.id));
    await saved.reloadFolders();
  } catch (e) {
    ui.say(`Could not remove the cover: ${err(e)}`);
  }
}

async function exportFolder(f: Folder) {
  try {
    if (await call((a) => a.features.saved.exportFolder(f.id))) ui.say(`Exported “${f.name}”. Anyone with Encore can import it.`);
  } catch (e) {
    ui.say(`Could not export: ${err(e)}`);
  }
}

export async function importFolder() {
  try {
    const f = await call((a) => a.features.saved.importFolder());
    if (!f) return;
    await saved.reloadFolders();
    catalog.addItems(f.items.map((i) => ({ type: "collection" as const, ...i.card })));
    ui.say(`Imported “${f.name}” with ${listsText(f.items.length)}`);
    nav.go({ name: "folder", id: f.id, title: f.name });
  } catch (e) {
    ui.say(err(e), "error");
  }
}

/** A folder's menu, in the sidebar and on its page. */
export function folderMenu(f: Folder, open = true): MenuItem[] {
  return [
    ...(open ? [{ label: "Open", icon: "folder", run: () => nav.go({ name: "folder", id: f.id, title: f.name }) }] : []),
    { label: "Rename…", icon: "edit", run: () => renameFolder(f) },
    { label: f.cover ? "Change cover…" : "Choose cover…", icon: "sparkle", run: () => void chooseCover(f) },
    ...(f.cover ? [{ label: "Remove cover", icon: "x", run: () => void clearCover(f) }] : []),
    { label: "Export…", icon: "external", run: () => void exportFolder(f) },
    { label: "Delete folder…", icon: "trash", danger: true, run: () => deleteFolder(f) },
  ];
}

/** "Add to folder…": the second menu. */
function pickFolder(id: string) {
  const now = saved.folderOf(id);
  ui.openMenuAgain([
    { label: "New folder…", icon: "plus", run: () => newFolder(id) },
    ...saved.folders.filter((f) => f !== now).slice(0, 14).map((f) => ({ label: f.name, icon: "folder", run: () => void fileIn(f, id) })),
  ]);
}

// ---------- hooks ----------

trackMenuItems.add(({ track }) => [{ label: trackBookmark(track.id) ? "Remove bookmark" : "Bookmark", icon: trackBookmark(track.id) ? "bookmarkFill" : "bookmark", run: () => toggleTrackBookmark(track) }], 40);

collectionMenuItems.add((id) => {
  const r: PageRef = { name: "collection", id, params: null };
  const f = saved.folderOf(id);
  const items: MenuItem[] = [{ label: pageBookmark(r) ? "Remove bookmark" : "Bookmark", icon: pageBookmark(r) ? "bookmarkFill" : "bookmark", run: () => togglePageBookmark(r) }];
  if (catalog.collection(id)) items.push({ label: f ? "Move to folder…" : "Add to folder…", icon: "folder", run: () => pickFolder(id) });
  if (f) items.push({ label: `Remove from “${f.name}”`, icon: "x", run: () => void unfile(f, id) });
  return items;
}, 40);

artistMenuItems.add((id) => {
  const r: PageRef = { name: "artist", id };
  return [{ label: pageBookmark(r) ? "Remove bookmark" : "Bookmark", icon: pageBookmark(r) ? "bookmarkFill" : "bookmark", run: () => togglePageBookmark(r) }];
}, 40);

deckMenuItems.add(() => (player.current ? [{ label: "Bookmark this moment", icon: "bookmark", run: bookmarkMoment }] : []), 40);

addKey({ key: "b", label: ["B", "Bookmark this moment of the song"], run: bookmarkMoment });

paletteItems.add((q) => {
  const out: PaletteItem[] = [];
  const cur = player.current;
  if (cur) out.push({ key: "c-moment", group: "Actions", icon: "bookmark", title: "Bookmark this moment", sub: `${fmt(player.position)} of “${cur.title}”`, hint: "B", run: bookmarkMoment });
  const r = pageRefOf(nav.view);
  if (r) out.push({ key: "c-page", group: "Actions", icon: pageBookmark(r) ? "bookmarkFill" : "bookmark", title: pageBookmark(r) ? "Remove bookmark of this page" : "Bookmark this page", sub: pageLabel(r).title, run: () => togglePageBookmark(r) });
  if (player.queue.length) out.push({ key: "c-saveq", group: "Actions", icon: "queue", title: "Save queue…", sub: `${songsText(player.queue.length)}, to restore later`, run: saveQueue });
  // Folders: found by name, and all listed under `#` (D32).
  for (const f of saved.folders) {
    out.push({ key: `c-f-${f.id}`, group: "Folders", icon: "folder", title: f.name, sub: listsText(f.items.length), hint: "Open", run: () => nav.go({ name: "folder", id: f.id, title: f.name }) });
  }
  if (!q) return out;
  // Typed: your bookmarks and saved queues are found too.
  for (const b of saved.bookmarks.slice(0, 200)) {
    out.push({ key: `c-b-${b.id}`, group: "Bookmarks", title: b.title, sub: bookmarkSub(b), art: b.art, round: b.target.type === "page" && b.target.value.name === "artist", hint: b.target.type === "page" ? "Open" : "Play", run: () => openBookmark(b) });
  }
  for (const s of saved.queues) {
    out.push({ key: `c-q-${s.id}`, group: "Saved queues", icon: "queue", title: s.name, sub: `${songsText(s.songs)} · restores paused`, hint: "Restore", run: () => void restoreQueue(s) });
  }
  return out;
}, 40);

registerFeature({ name: "saved", load: () => saved.load() });
