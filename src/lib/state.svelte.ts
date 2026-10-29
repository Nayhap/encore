import { SvelteSet } from "svelte/reactivity";
import { BackendError, call, type AuthState, type Lyrics, type Phase, type PlayerSnapshot, type PositionAnchor, type QueueEntry, type QueueSource, type QueueSplice, type Repeat, type Track } from "./backend";
import { catalog } from "./catalog.svelte";

// The player is a mirror of the Rust playback state (D6, D13). Its public
// methods are the UI contract; each one becomes a backend command. Position
// arrives as 2 Hz anchors and is interpolated with requestAnimationFrame.

export type RepeatMode = Repeat;

const big = (s: string | null | undefined) => {
  try {
    return BigInt(s ?? "0");
  } catch {
    return 0n;
  }
};

class Player {
  queue = $state<QueueEntry[]>([]);
  index = $state<number | null>(null);
  position = $state(0);
  playing = $state(false);
  phase = $state<Phase>("idle");
  shuffle = $state(false);
  repeat = $state<RepeatMode>("off");
  volume = $state(0.7);
  muted = $state(false);
  autoplay = $state(true);
  duration = $state<number | null>(null);
  /** Where the queue came from, shown as "Playing from …". */
  source = $state<QueueSource | null>(null);
  error = $state<string | null>(null);
  ready = $state(false);
  liked = new SvelteSet<string>();
  lyrics = $state<Lyrics | null>(null);
  lyricsFor = $state<string | null>(null);

  current: Track | undefined = $derived(this.index != null ? this.queue[this.index]?.track : undefined);
  currentEntry: QueueEntry | undefined = $derived(this.index != null ? this.queue[this.index] : undefined);
  upNext = $derived(this.index != null ? this.queue.slice(this.index + 1) : this.queue);
  /** Lyrics belonging to the current track, if loaded. */
  currentLyrics = $derived(this.current && this.lyricsFor === this.current.id ? this.lyrics : null);
  timedLines = $derived(this.currentLyrics && this.currentLyrics.lines.length ? this.currentLyrics.lines : null);

  #rev = -1n;
  #qrev = "0";
  #trackGen = 0n;
  #seekGen = 0n;
  #localSeek = -1n;
  #anchor = { pos: 0, at: 0, playing: false };
  #raf = 0;

  constructor() {
    if (typeof window !== "undefined") this.#tick();
  }

  #tick = () => {
    this.#raf = requestAnimationFrame(this.#tick);
    if (document.hidden) return;
    const a = this.#anchor;
    if (!a.playing) return;
    const dt = Math.min((performance.now() - a.at) / 1000, 2);
    let p = a.pos + dt;
    if (this.duration != null) p = Math.min(p, this.duration);
    this.position = p;
  };

  // ---------- applying backend state ----------

  applySnapshot(s: PlayerSnapshot) {
    const r = big(s.revision);
    if (r <= this.#rev && this.#rev >= 0n) return;
    this.#rev = r;
    const tg = big(s.trackGen);
    const trackChanged = tg !== this.#trackGen;
    this.#trackGen = tg;
    if (s.current) {
      catalog.addTracks([s.current.track]);
      if (s.current.track.liked === true) this.liked.add(s.current.track.id);
    }
    // The snapshot's entry is authoritative for the current index.
    if (s.index != null && s.current && this.queue[s.index]?.uid !== s.current.uid) {
      void this.resyncQueue();
    }
    this.index = s.index;
    this.phase = s.phase;
    this.playing = s.playing;
    this.shuffle = s.shuffle;
    this.repeat = s.repeat;
    this.autoplay = s.autoplay;
    this.volume = s.volume ?? 0.7;
    this.muted = s.muted;
    this.source = s.source;
    this.duration = s.duration;
    this.error = s.error?.message ?? null;
    const sg = big(s.seekGen);
    if (trackChanged || sg > this.#seekGen || this.#localSeek < 0n) {
      this.#seekGen = sg;
      this.#setAnchor(s.position ?? 0, s.playing && s.phase === "playing");
    } else {
      this.#anchor.playing = s.playing && s.phase === "playing";
      if (!this.#anchor.playing) this.position = this.#anchor.pos;
    }
    this.ready = true;
  }

  applyAnchor(a: PositionAnchor) {
    const tg = big(a.trackGen);
    const sg = big(a.seekGen);
    if (tg < this.#trackGen || sg < this.#seekGen) return;
    if (this.#localSeek >= 0n && sg <= this.#localSeek) return; // our seek not applied yet
    this.#localSeek = -1n;
    this.#trackGen = tg;
    this.#seekGen = sg;
    if (a.duration != null) this.duration = a.duration;
    this.#setAnchor(a.position ?? 0, a.playing);
  }

  #setAnchor(pos: number, playing: boolean) {
    this.#anchor = { pos, at: performance.now(), playing };
    this.position = pos;
  }

  applySplice(q: QueueSplice) {
    if (q.base !== this.#qrev) {
      void this.resyncQueue();
      return;
    }
    const next = [...this.queue];
    next.splice(q.start, q.delete, ...q.insert);
    this.queue = next;
    this.#qrev = q.revision;
    this.index = q.index;
    catalog.addTracks(q.insert.map((e) => e.track));
    for (const e of q.insert) if (e.track.liked === true) this.liked.add(e.track.id);
  }

  setQueue(entries: QueueEntry[], revision: string, index: number | null) {
    this.queue = entries;
    this.#qrev = revision;
    this.index = index;
    catalog.addTracks(entries.map((e) => e.track));
    for (const e of entries) if (e.track.liked === true) this.liked.add(e.track.id);
  }

  async resyncQueue() {
    const q = await call((a) => a.queueSnapshot()).catch(() => null);
    if (q) this.setQueue(q.entries, q.revision, q.index);
  }

  setLyrics(trackId: string, l: Lyrics | null) {
    this.lyricsFor = trackId;
    this.lyrics = l;
  }

  // ---------- commands ----------

  #run(f: Parameters<typeof call>[0], fail = "That did not work") {
    call(f).catch((e: BackendError) => ui.say(`${fail}: ${e.message}`));
  }

  #tracks(ids: string[]) {
    return ids.map((id) => catalog.track(id)).filter((t): t is Track => !!t);
  }

  playTracks(tracks: Track[], start = 0, source: QueueSource | null = null, shuffle = false) {
    if (!tracks.length) return;
    catalog.addTracks(tracks);
    this.#run((a) => a.playTracks(tracks, Math.min(start, tracks.length - 1), source, shuffle), "Could not play");
  }

  playIds(ids: string[], start = 0, source: { label: string; collectionId?: string } | null = null, shuffle = false) {
    const tracks = this.#tracks(ids);
    this.playTracks(tracks, start, source ? { label: source.label, collectionId: source.collectionId ?? null, radioPlaylist: null } : null, shuffle);
  }

  playCollection(id: string, start = 0, shuffle = false) {
    this.#run((a) => a.playCollection(id, start, shuffle), "Could not play");
  }

  /** With context, play that list from the track; without, start its radio. */
  playTrack(id: string, context?: { ids: string[]; label: string; collectionId?: string }) {
    if (context) {
      const tracks = this.#tracks(context.ids);
      const start = Math.max(0, tracks.findIndex((t) => t.id === id));
      this.playTracks(tracks, start, { label: context.label, collectionId: context.collectionId ?? null, radioPlaylist: null });
    } else {
      const t = catalog.track(id);
      if (t) this.#run((a) => a.playTrack(t), "Could not play");
    }
  }

  startRadio(playlistId: string, label: string) {
    this.#run((a) => a.startRadio(playlistId, label), "Could not start the radio");
  }

  toggle() {
    if (!this.current) return;
    // Optimistic: flip immediately, the next snapshot confirms.
    this.playing = !this.playing;
    this.#anchor = { pos: this.position, at: performance.now(), playing: this.playing && this.phase === "playing" };
    this.#run((a) => a.toggle());
  }

  next() {
    this.#run((a) => a.next());
  }

  prev() {
    this.#run((a) => a.prev());
  }

  seek(t: number) {
    if (!this.current || !Number.isFinite(t)) return;
    const d = this.duration ?? this.current.duration ?? 0;
    t = Math.min(Math.max(0, t), Math.max(0, d - 0.25));
    this.#localSeek = this.#seekGen;
    this.#setAnchor(t, this.#anchor.playing);
    this.#run((a) => a.seek(t));
  }

  jumpTo(queueIndex: number) {
    const e = this.queue[queueIndex];
    if (e) this.#run((a) => a.jumpTo(e.uid));
  }

  playNext(id: string | string[]) {
    const tracks = this.#tracks(Array.isArray(id) ? id : [id]);
    if (tracks.length) this.#run((a) => a.playNext(tracks));
  }

  addToQueue(id: string | string[]) {
    const tracks = this.#tracks(Array.isArray(id) ? id : [id]);
    if (tracks.length) this.#run((a) => a.addToQueue(tracks));
  }

  removeAt(queueIndex: number) {
    const e = this.queue[queueIndex];
    if (!e || queueIndex === this.index) return;
    this.#run((a) => a.queueRemove(e.uid));
  }

  move(from: number, to: number) {
    if (from === to || from === this.index) return;
    const e = this.queue[from];
    if (!e) return;
    // Dropping onto `to` places the entry where `to` was.
    const rest = this.queue.filter((_, i) => i !== from);
    const before = rest[to]?.uid ?? null;
    this.#run((a) => a.queueMove(e.uid, before));
  }

  clearUpNext() {
    this.#run((a) => a.clearUpNext());
  }

  toggleShuffle() {
    this.shuffle = !this.shuffle;
    const on = this.shuffle;
    this.#run((a) => a.setShuffle(on));
  }

  cycleRepeat() {
    this.repeat = this.repeat === "off" ? "all" : this.repeat === "all" ? "one" : "off";
    const m = this.repeat;
    this.#run((a) => a.setRepeat(m));
  }

  setAutoplay(on: boolean) {
    this.autoplay = on;
    this.#run((a) => a.setAutoplay(on));
  }

  toggleLike(id: string) {
    const was = this.liked.has(id);
    if (was) this.liked.delete(id);
    else this.liked.add(id);
    call((a) => a.setLike(id, !was))
      .then((r) => {
        if (r === "rejected") {
          if (was) this.liked.add(id);
          else this.liked.delete(id);
          ui.say("YouTube Music did not accept that change");
        } else if (r === "outcomeUnknown") ui.say("Not sure that was saved; check again in a moment");
      })
      .catch((e: BackendError) => {
        if (was) this.liked.add(id);
        else this.liked.delete(id);
        ui.say(e.code === "authRequired" ? "Sign in to like songs" : `Could not change the like: ${e.message}`);
      });
  }

  setVolume(v: number) {
    this.volume = Math.min(1, Math.max(0, v));
    if (this.volume > 0) this.muted = false;
    const x = this.volume;
    this.#run((a) => a.setVolume(x));
  }

  setMuted(m: boolean) {
    this.muted = m;
    this.#run((a) => a.setMuted(m));
  }
}

export type View =
  | { name: "home" }
  | { name: "explore" }
  | { name: "library" }
  | { name: "collection"; id: string }
  | { name: "artist"; id: string }
  | { name: "page"; browseId: string; params?: string | null; title?: string }
  | { name: "settings" };

class Nav {
  stack = $state<View[]>([{ name: "home" }]);
  i = $state(0);
  view = $derived(this.stack[this.i]);
  canBack = $derived(this.i > 0);
  canForward = $derived(this.i < this.stack.length - 1);

  go(v: View) {
    const cur = this.view;
    if (JSON.stringify(cur) === JSON.stringify(v)) return;
    this.stack = [...this.stack.slice(0, this.i + 1), v];
    this.i = this.stack.length - 1;
  }
  back() {
    if (this.canBack) this.i--;
  }
  forward() {
    if (this.canForward) this.i++;
  }
}

export interface MenuItem {
  label: string;
  icon?: string;
  run: () => void;
  danger?: boolean;
}

export type Theme = "system" | "dark" | "light";

class UI {
  stageOpen = $state(true);
  theater = $state(false);
  palette = $state(false);
  shortcuts = $state(false);
  theme = $state<Theme>((localStorage.getItem("theme") as Theme) ?? "system");
  tint = $state(localStorage.getItem("tint") !== "off");
  menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  toast = $state<{ text: string; id: number } | null>(null);

  say(text: string) {
    const id = Date.now();
    this.toast = { text, id };
    setTimeout(() => {
      if (this.toast?.id === id) this.toast = null;
    }, 3200);
  }

  lastMenuAt = { x: 200, y: 200 };

  openMenu(e: MouseEvent, items: MenuItem[]) {
    e.preventDefault();
    e.stopPropagation();
    const el = e.currentTarget as HTMLElement | null;
    // Keyboard-opened menus have no pointer position; anchor them to the control.
    if (e.clientX === 0 && e.clientY === 0 && el) {
      const r = el.getBoundingClientRect();
      this.menu = { x: r.left, y: r.bottom + 4, items };
    } else {
      this.menu = { x: e.clientX, y: e.clientY, items };
    }
    this.lastMenuAt = { x: this.menu.x, y: this.menu.y };
  }

  /** Open a follow-up menu where the last one was. */
  openMenuAgain(items: MenuItem[]) {
    this.menu = { ...this.lastMenuAt, items };
  }
}

class Account {
  state = $state<AuthState>({ state: "anonymous" });
  signin = $state<{ stage: string; message: string | null } | null>(null);
  signedIn = $derived(this.state.state === "signedIn");
  name = $derived(this.state.state === "signedIn" ? this.state.channel.name : null);
}

export const player = new Player();
export const nav = new Nav();
export const ui = new UI();
export const account = new Account();

const remote = (id: string) => id.replace(/^(ytm|mock):/, "");

/** Shared track actions so every menu uses the same words for the same thing. */
export function trackMenu(id: string): MenuItem[] {
  const tr = catalog.track(id);
  if (!tr) return [];
  const liked = player.liked.has(id);
  const artist = tr.artists.find((a) => a.id);
  const items: MenuItem[] = [
    { label: "Play next", icon: "playNext", run: () => (player.playNext(id), ui.say(`“${tr.title}” plays next`)) },
    { label: "Add to queue", icon: "queue", run: () => (player.addToQueue(id), ui.say(`Added “${tr.title}” to queue`)) },
    { label: "Start radio", icon: "radio", run: () => player.playTrack(id) },
    { label: liked ? "Remove from liked music" : "Add to liked music", icon: liked ? "heartFill" : "heart", run: () => (player.toggleLike(id), ui.say(liked ? "Removed from liked music" : "Added to liked music")) },
  ];
  if (account.signedIn) items.push({ label: "Add to playlist…", icon: "plus", run: () => void pickPlaylist(id) });
  if (artist?.id) items.push({ label: "Go to artist", icon: "artist", run: () => nav.go({ name: "artist", id: artist.id! }) });
  if (tr.album?.id) items.push({ label: "Go to album", icon: "album", run: () => nav.go({ name: "collection", id: tr.album!.id! }) });
  items.push({ label: "Copy link", icon: "link", run: () => (navigator.clipboard?.writeText(`https://music.youtube.com/watch?v=${remote(id)}`), ui.say("Link copied")) });
  return items;
}

export function collectionMenu(id: string): MenuItem[] {
  const c = catalog.collection(id);
  const title = c?.title ?? "Collection";
  const withTracks = async (f: (ids: string[]) => void) => {
    const full = await call((a) => a.collection(id)).catch(() => null);
    if (!full) return ui.say("Could not load that collection");
    catalog.addCollection(full);
    f(full.entries.map((e) => e.track.id));
  };
  const listId = remote(id).replace(/^VL/, "");
  return [
    { label: "Play", icon: "play", run: () => player.playCollection(id) },
    { label: "Shuffle", icon: "shuffle", run: () => player.playCollection(id, 0, true) },
    { label: "Play next", icon: "playNext", run: () => withTracks((ids) => (player.playNext(ids), ui.say(`“${title}” plays next`))) },
    { label: "Add to queue", icon: "queue", run: () => withTracks((ids) => (player.addToQueue(ids), ui.say(`Added “${title}” to queue`))) },
    {
      label: "Copy link",
      icon: "link",
      run: () => (navigator.clipboard?.writeText(listId.startsWith("MPRE") ? `https://music.youtube.com/browse/${listId}` : `https://music.youtube.com/playlist?list=${listId}`), ui.say("Link copied")),
    },
  ];
}

/** Second menu: the user's playlists, to add a song to. */
async function pickPlaylist(trackId: string) {
  const lib = await call((a) => a.library("playlists")).catch(() => null);
  const lists = (lib?.items ?? []).filter((i) => i.type === "collection" && i.kind !== "liked") as { id: string; title: string }[];
  if (!lists.length) return ui.say("No playlists to add to");
  ui.openMenuAgain(
    lists.slice(0, 14).map((p) => ({
      label: p.title,
      icon: "list",
      run: () => {
        const pid = p.id.replace(/^(ytm|mock):/, "");
        call((a) => a.playlistAdd(pid, [trackId]))
          .then((r) => ui.say(r === "done" ? `Added to “${p.title}”` : r === "outcomeUnknown" ? "Not sure that worked; check the playlist" : "YouTube Music did not accept that"))
          .catch((e: BackendError) => ui.say(`Could not add: ${e.message}`));
      },
    })),
  );
}

/** Play an artist: their radio if YouTube offers one, else the top songs. */
export async function playArtist(id: string, shuffle = false) {
  const a = await call((x) => x.artist(id)).catch((e: BackendError) => (ui.say(`Could not load the artist: ${e.message}`), null));
  if (!a) return;
  catalog.addArtist(a);
  const radio = shuffle ? a.shufflePlaylist ?? a.radioPlaylist : a.radioPlaylist ?? a.shufflePlaylist;
  if (radio) player.startRadio(radio, a.name);
  else player.playTracks(a.topSongs, 0, { label: a.name, collectionId: null, radioPlaylist: null }, shuffle);
}

/** Connect to the backend: subscribe first, then bootstrap (D13). */
export async function connect() {
  await call((a) =>
    a.subscribe({
      playerState: (s) => player.applySnapshot(s),
      playerPosition: (p) => player.applyAnchor(p),
      queue: (q) => player.applySplice(q),
      auth: (s) => (account.state = s),
      lyrics: (l) => player.setLyrics(l.trackId, l.lyrics),
      artHue: (h) => catalog.hues.set(h.key, [h.h1, h.h2]),
      notice: (n) => ui.say(n.text),
      signin: (s) => (account.signin = { stage: s.stage, message: s.message }),
    }),
  );
  const b = await call((a) => a.bootstrap());
  account.state = b.auth;
  player.setQueue(b.queue.entries, b.queue.revision, b.queue.index);
  player.applySnapshot(b.player);
  // Lyrics for a restored track.
  const cur = player.current;
  if (cur && player.lyricsFor !== cur.id) {
    call((a) => a.lyrics(cur)).then((l) => player.setLyrics(cur.id, l)).catch(() => {});
  }
}

export { catalog };
