import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { BackendError, call, isTauri, type AuthState, type CollectionCard, type Counterpart, type DownloadItem, type DownloadItemEvent, type DownloadsSnapshot, type DownloadsSummary, type KeptCollection, type LoopRange, type Lyrics, type MutationOutcome, type Phase, type PlayerSnapshot, type PositionAnchor, type Privacy, type PrivateSession, type QueueEntry, type QueueSource, type QueueSplice, type Repeat, type SleepTimer, type Track, type UpdateStatus } from "./backend";
import { catalog } from "./catalog.svelte";
import { sizeText } from "./data";
import { collectionMenuItems, grouped, loadFeatures, subscribeFeatures, trackMenuItems, type MenuItem, type TrackBand } from "./features/hooks";

// The player is a mirror of the Rust playback state (D6, D13). Its public
// methods are the UI contract; each one becomes a backend command. Position
// arrives as 2 Hz anchors and is interpolated ten times a second: the seek
// bar then moves less than a pixel a step, and the window isn't redrawn every
// frame (on Linux, WebKitGTK may paint every frame on the CPU).

/** How often the interpolated position is written while playing. */
const TICK_MS = 100;

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
  /** No audio output works: playback is silent until one does. */
  noOutput = $state(false);
  ready = $state(false);
  liked = new SvelteSet<string>();
  lyrics = $state<Lyrics | null>(null);
  lyricsFor = $state<string | null>(null);
  /** The snapshot's reserved fields (lanes A and D fill them in Rust). */
  abLoop = $state<LoopRange | null>(null);
  sleep = $state<SleepTimer | null>(null);
  privateSession = $state<PrivateSession | null>(null);
  /** When `privateSession` arrived: its `remaining` counts from here. */
  privateSessionAt = 0;

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

  constructor() {
    if (typeof window === "undefined") return;
    setInterval(this.#tick, TICK_MS);
    // A hidden window keeps the anchors but writes no position, so nothing
    // is restyled for it; it catches up once when it is shown again.
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) this.#show();
    });
  }

  /** The position the anchor gives for now. */
  #show() {
    const a = this.#anchor;
    if (a.playing) this.#tick();
    else this.position = a.pos;
  }

  #tick = () => {
    if (document.hidden) return;
    const a = this.#anchor;
    if (!a.playing) return;
    const dt = Math.min((performance.now() - a.at) / 1000, 2);
    let p = a.pos + dt;
    // Inside an A–B loop the engine wraps at B (D26, lane A).
    const l = this.abLoop;
    if (l?.a != null && l.b != null && a.pos < l.b && p >= l.b) p = l.a + ((p - l.b) % Math.max(l.b - l.a, 0.001));
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
    this.noOutput = s.noOutput ?? false;
    this.abLoop = s.loop ?? null;
    this.sleep = s.sleep ?? null;
    this.privateSession = s.private ?? null;
    this.privateSessionAt = Date.now();
    const sg = big(s.seekGen);
    if (trackChanged || sg > this.#seekGen || this.#localSeek < 0n) {
      this.#seekGen = sg;
      this.#setAnchor(s.position ?? 0, s.playing && s.phase === "playing");
    } else {
      this.#anchor.playing = s.playing && s.phase === "playing";
      if (!this.#anchor.playing && !document.hidden) this.position = this.#anchor.pos;
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
    if (!document.hidden) this.position = pos;
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

  /** `params` when the link carries them (an artist's "All songs"). */
  playCollection(id: string, start = 0, shuffle = false, params: string | null = null) {
    this.#run((a) => a.playCollection(id, start, shuffle, params), "Could not play");
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
  | { name: "collection"; id: string; params?: string | null }
  | { name: "artist"; id: string }
  | { name: "page"; browseId: string; params?: string | null; title?: string }
  | { name: "history" }
  | { name: "downloads" }
  | { name: "settings" }
  // Lane C: a folder of playlists and albums (D34).
  | { name: "folder"; id: string; title?: string };

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

export type { MenuItem };

/** A button on a toast. */
export interface ToastAction {
  label: string;
  run: () => void;
}


export type Theme = "system" | "dark" | "light";

/** The create / edit playlist sheet. */
export interface PlaylistSheet {
  mode: "create" | "edit";
  /** Editing: the playlist (`ytm:VL…`) and its bare id. */
  id?: string;
  playlistId?: string;
  title?: string;
  description?: string;
  privacy?: Privacy;
  /** Creating from "Add to playlist": the songs it starts with. */
  trackIds?: string[];
  /** Creating a copy of a list ("Save as playlist…"): the list and its name. */
  copyOf?: string;
  sourceTitle?: string;
}

/** A yes/no question before something that can't be undone. */
export interface Confirm {
  title: string;
  text: string;
  confirm: string;
  danger?: boolean;
  run: () => void;
}

class UI {
  stageOpen = $state(true);
  theater = $state(false);
  palette = $state(false);
  shortcuts = $state(false);
  theme = $state<Theme>((localStorage.getItem("theme") as Theme) ?? "system");
  tint = $state(localStorage.getItem("tint") !== "off");
  /** Settings → Appearance: fill the current lyric line as it is sung, or
   *  just highlight it. */
  lyricFill = $state(localStorage.getItem("lyric-fill") !== "off");
  /** Windows and Linux: use the OS title bar instead of Encore's buttons. */
  systemTitlebar = $state(localStorage.getItem("titlebar") === "system");
  menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  toast = $state<{ text: string; id: number; ms: number; level: "info" | "warn" | "error"; action: ToastAction | null } | null>(null);
  sheet = $state<PlaylistSheet | null>(null);
  confirm = $state<Confirm | null>(null);
  /** Settings → Playback; Rust owns the value (loaded at bootstrap). */
  reportHistory = $state(true);
  /** Bumps when a play was reported, so History can refresh. */
  historyRev = $state(0);
  /** Encore can reach YouTube; Rust decides (D23). */
  online = $state(true);

  /** A short message above the player bar. Longer messages (errors that
   *  say what to do) stay up long enough to read, and one with an action
   *  long enough to use it. */
  say(text: string, level: "info" | "warn" | "error" = "info", action: ToastAction | null = null) {
    const id = Date.now();
    const ms = Math.min(14000, Math.max(2400, 1200 + text.length * 55)) + (level === "info" ? 0 : 2000) + (action ? 5000 : 0);
    this.toast = { text, id, ms, level, action };
    setTimeout(() => {
      if (this.toast?.id === id) this.toast = null;
    }, ms + 400);
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

/** Your playlists (the library's playlist list), shared by the sidebar,
 *  Library and "Add to playlist". `rev` bumps on every change so open views
 *  refetch (D20). */
class Playlists {
  list = $state<CollectionCard[]>([]);
  rev = $state(0);
  loaded = $state(false);
  #seq = 0;

  async load() {
    const my = ++this.#seq;
    if (!account.signedIn) {
      this.list = [];
      this.loaded = false;
      return;
    }
    const p = await call((a) => a.library("playlists")).catch(() => null);
    if (my !== this.#seq || !p) return;
    catalog.addItems(p.items);
    this.list = p.items.filter((i) => i.type === "collection") as CollectionCard[];
    this.loaded = true;
  }

  /** The playlist the last change was about (`null`: not known). */
  last = $state<string | null>(null);

  /** Something changed on the server: refetch everywhere. */
  changed(id: string | null = null) {
    this.last = id;
    this.rev++;
    void this.load();
  }

  /** Show a playlist straight away, before the refetch brings its art. */
  insert(c: CollectionCard) {
    if (!this.list.some((x) => x.id === c.id)) this.list = [c, ...this.list.filter((x) => x.kind === "liked"), ...this.list.filter((x) => x.kind !== "liked")].filter((x, i, a) => a.findIndex((y) => y.id === x.id) === i);
  }

  remove(id: string) {
    this.list = this.list.filter((x) => x.id !== id);
  }
}

class Account {
  state = $state<AuthState>({ state: "anonymous" });
  signin = $state<{ stage: string; message: string | null } | null>(null);
  signedIn = $derived(this.state.state === "signedIn");
  name = $derived(this.state.state === "signedIn" ? this.state.channel.name : null);
}

/** In-app updates (D24). Rust owns the state; this mirrors it, says once
 *  per version per session that one is out or ready, and turns actions into
 *  commands. Nothing here restarts Encore by itself. */
class Updates {
  status = $state<UpdateStatus | null>(null);
  /** The update sheet: the new version, or "What's new" after an update. */
  sheet = $state<"update" | "whatsNew" | null>(null);
  #rev = -1n;
  #told = new Set<string>();
  /** The version the user asked to download: its failure is said once. */
  #asked: string | null = null;

  phase = $derived(this.status?.phase ?? "idle");
  ready = $derived(this.phase === "ready");
  installing = $derived(this.phase === "installing");
  version = $derived(this.status?.available?.version ?? null);

  apply(s: UpdateStatus) {
    const r = big(s.revision);
    if (r < this.#rev) return;
    this.#rev = r;
    this.status = s;
    this.#tell(s);
  }

  #once(key: string, f: () => void) {
    if (this.#told.has(key)) return;
    this.#told.add(key);
    f();
  }

  /** The quiet notices: an update is ready, or out when Encore won't fetch it. */
  #tell(s: UpdateStatus) {
    const a = s.available;
    if (s.updated) {
      const v = s.updated.version;
      this.#once(`updated:${v}`, () => ui.say(`Encore is updated to ${v}.`, "info", { label: "What's new", run: () => this.whatsNew() }));
    }
    if (!a) return;
    if (s.phase === "failed" && this.#asked === a.version) {
      this.#asked = null;
      ui.say(`Couldn't download Encore ${a.version}: ${s.error ?? "it stopped"}.`, "error", { label: "Try again", run: () => void this.download() });
    } else if (s.phase !== "downloading") this.#asked = null;
    if (s.phase === "ready") {
      this.#once(`ready:${a.version}`, () => ui.say(s.atQuit ? `Encore ${a.version} is ready. It installs when you quit Encore.` : `Encore ${a.version} is ready to install.`, "info", { label: "Restart now", run: () => void this.install() }));
    } else if (s.phase === "available" && s.skipped !== a.version && !(s.auto && s.canInstall)) {
      this.#once(`out:${a.version}`, () => this.#out(s));
    }
  }

  #out(s: UpdateStatus) {
    const a = s.available!;
    if (s.install === "deb" || s.install === "rpm") ui.say(`Encore ${a.version} is out. Download it from the releases page.`, "info", { label: "Open page", run: () => openExternal(a.page) });
    else if (!s.canInstall) ui.say(`Encore ${a.version} is out.`, "info", { label: "What's new", run: () => (this.sheet = "update") });
    else ui.say(`Encore ${a.version} is out.`, "info", { label: "Download", run: () => void this.download() });
  }

  whatsNew() {
    const u = this.status?.updated;
    if (u && !u.notes.length) openExternal(u.page);
    else this.sheet = "whatsNew";
  }

  /** "Check for updates": always says what GitHub answered. */
  async check() {
    if (this.status?.install === "dev") return ui.say(this.status.reason ?? "Updates are off in development builds.");
    const s = await call((a) => a.updateCheck()).catch((e: BackendError) => (ui.say(`Couldn't check for updates: ${e.message}`, "error"), null));
    if (!s) return;
    this.apply(s);
    const a = s.available;
    if (s.phase === "upToDate") ui.say(`Encore ${s.current} is up to date.`);
    else if (s.error && s.phase !== "ready") ui.say(`Couldn't check for updates: ${s.error}. Check your connection and try again.`, "error");
    else if (a && s.phase === "downloading") ui.say(`Encore ${a.version} is out. Downloading it now.`);
    else if (a && s.phase === "ready") ui.say(`Encore ${a.version} is ready to install.`, "info", { label: "Restart now", run: () => void this.install() });
    else if (a) {
      this.#told.add(`out:${a.version}`);
      this.#out(s);
    }
  }

  async download() {
    this.#asked = this.status?.available?.version ?? null;
    const s = await call((a) => a.updateDownload()).catch((e: BackendError) => (ui.say(`Couldn't download the update: ${e.message}`, "error"), null));
    if (s) this.apply(s);
  }

  /** Pause, install and restart into the new version (Rust saves the queue
   *  and position first). Only comes back if the install failed. */
  async install() {
    this.sheet = null;
    await call((a) => a.updateInstall()).catch((e: BackendError) => ui.say(e.message, "error"));
  }

  async skip(version: string) {
    this.sheet = null;
    const s = await call((a) => a.updateSkip(version)).catch((e: BackendError) => (ui.say(`Could not skip it: ${e.message}`, "error"), null));
    if (!s) return;
    this.apply(s);
    ui.say(`Skipped Encore ${version}. Encore tells you when a newer version is out.`);
  }

  async unskip() {
    const s = await call((a) => a.updateUnskip()).catch((e: BackendError) => (ui.say(`Could not change that: ${e.message}`, "error"), null));
    if (s) this.apply(s);
  }

  async setAuto(on: boolean) {
    const s = await call((a) => a.setAutoUpdate(on)).catch((e: BackendError) => (ui.say(`Could not change that: ${e.message}`, "error"), null));
    if (s) this.apply(s);
  }
}

/** Playlists are kept under their `VL…` id, whichever form a view has. */
const keptKey = (id: string) => {
  const m = /^ytm:(.*)$/.exec(id);
  if (!m || /^(MPRE|VL|FE)/.test(m[1])) return id;
  return `ytm:VL${m[1]}`;
};

/** A summary without its progress, to tell a real change from a tick. */
const settled = ({ active, bytes: _bytes, ...rest }: DownloadsSummary) => JSON.stringify({ ...rest, active: active?.trackId ?? null });

const songs = (n: number) => (n === 1 ? "1 song" : `${n} songs`);

/** Downloads (D23). Rust owns them; this mirrors the songs, the kept
 *  collections and the summary, and turns actions into commands. Item
 *  events carry the revision they made: a gap means one was missed, and
 *  the snapshot is fetched again. */
class Downloads {
  items = new SvelteMap<string, DownloadItem>();
  kept = $state<KeptCollection[]>([]);
  summary = $state<DownloadsSummary | null>(null);
  #rev = -1n;
  #loading: Promise<void> | null = null;
  /** Something changed while a snapshot was on its way: fetch another. */
  #again = false;
  /** The newest revision an event announced while a snapshot was loading. */
  #seen = -1n;
  /** The last summary that came meanwhile: pause and waits change it
   *  without a new revision. */
  #pending: DownloadsSummary | null = null;
  #countsTimer = 0;

  available = $derived(this.summary?.available ?? false);
  /** Songs on this computer. */
  count = $derived(this.summary?.songs ?? 0);
  /** Something is downloading, waiting or being checked. */
  busy = $derived(!!this.summary && (this.summary.waiting > 0 || this.summary.syncing.length > 0));

  has(id: string) {
    return this.items.get(id)?.state === "ready";
  }

  /** What can play now: anything online; offline, only downloads. */
  playable(t: Track) {
    return t.playable && (ui.online || this.has(t.id));
  }

  /** 0 to 1 while `id` downloads and its size is known. */
  progress(id: string): number | null {
    const a = this.summary?.active;
    if (!a || a.trackId !== id) return null;
    const it = this.items.get(id);
    if (it && it.attempt !== a.attempt) return null;
    return a.total ? Math.min(1, (a.bytes ?? 0) / a.total) : null;
  }

  /** The kept copy of a collection, this account's first. */
  keptFor(id: string): KeptCollection | undefined {
    const k = keptKey(id);
    const all = this.kept.filter((c) => keptKey(c.card.id) === k);
    return all.find((c) => c.mine) ?? all[0];
  }

  syncing(id: string) {
    const k = keptKey(id);
    return !!this.summary?.syncing.some((s) => keptKey(s) === k);
  }

  apply(s: DownloadsSnapshot) {
    // Older than events already applied: keep those, and ask again.
    if (big(s.summary.revision) < this.#rev) {
      this.#again = true;
      return;
    }
    this.#rev = big(s.summary.revision);
    this.summary = s.summary;
    this.items.clear();
    for (const it of s.items) this.items.set(it.track.id, it);
    catalog.addTracks(s.items.map((i) => i.track));
    this.kept = s.collections;
  }

  refresh(): Promise<void> {
    if (this.#loading) {
      this.#again = true;
      return this.#loading;
    }
    this.#again = false;
    this.#loading = call((a) => a.downloadsSnapshot())
      .then((s) => this.apply(s))
      .catch(() => {})
      .finally(() => {
        this.#loading = null;
        // A summary at the same revision may be older or newer than the
        // snapshot; unless only progress differs, ask once more.
        const p = this.#pending;
        this.#pending = null;
        const differs = !!p && !!this.summary && big(p.revision) === this.#rev && settled(p) !== settled(this.summary);
        if (this.#again || differs || this.#seen > this.#rev) void this.refresh();
      });
    return this.#loading;
  }

  /** Kept collections' counts come with snapshots: one, shortly after
   *  songs finish, keeps their progress current. */
  #countsSoon() {
    if (!this.kept.length || this.#countsTimer) return;
    this.#countsTimer = window.setTimeout(() => {
      this.#countsTimer = 0;
      void this.refresh();
    }, 1500);
  }

  onSummary(s: DownloadsSummary) {
    const r = big(s.revision);
    if (this.#rev < 0n || this.#loading) {
      // Not caught up yet: the snapshot on its way may be older than this.
      // Progress at the same revision needs no other snapshot.
      if (r > this.#seen) this.#seen = r;
      this.#pending = s;
      return;
    }
    if (r < this.#rev) return;
    this.summary = s;
    if (r > this.#rev) void this.refresh();
  }

  onItem(e: DownloadItemEvent) {
    const r = big(e.revision);
    if (this.#rev < 0n || this.#loading) {
      if (r > this.#seen) this.#seen = r;
      return;
    }
    if (r <= this.#rev) return;
    if (r !== this.#rev + 1n) return void this.refresh();
    this.#rev = r;
    const it = e.item;
    if (it.state === "removed") this.items.delete(it.track.id);
    else {
      this.items.set(it.track.id, it);
      catalog.addTracks([it.track]);
    }
    if (it.state !== "queued" && it.state !== "downloading") this.#countsSoon();
  }

  #off() {
    if (this.available) return false;
    ui.say(this.summary?.message ?? "Downloads aren't available on this computer.", "error");
    return true;
  }

  async download(ids: string[]) {
    if (this.#off()) return;
    const tracks = ids.map((id) => catalog.track(id)).filter((t): t is Track => !!t && t.playable);
    if (!tracks.length) return ui.say("These songs can't be downloaded");
    try {
      await call((a) => a.downloadTracks(tracks));
      const what = tracks.length === 1 ? `“${tracks[0].title}”` : songs(tracks.length);
      ui.say(ui.online ? `Downloading ${what}` : `${what[0].toUpperCase()}${what.slice(1)} will download when you're back online`);
    } catch (e: any) {
      ui.say(`Could not download: ${e.message}`);
    }
  }

  /** Remove downloads (or stop them while they download). */
  async remove(ids: string[], stopping = false) {
    try {
      const r = await call((a) => a.downloadRemove(ids));
      const one = ids.length === 1 ? (catalog.track(ids[0])?.title ?? "the song") : null;
      if (r.kept.length) {
        const k = r.kept[0];
        const t = catalog.track(k.trackId)?.title ?? "This song";
        ui.say(k.collection ? `“${t}” stays downloaded because “${k.collection}” includes it.` : `“${t}” stays downloaded because another account on this computer downloaded it too.`);
      } else if (one) ui.say(stopping ? `Stopped downloading “${one}”` : `Removed “${one}” from downloads`);
      else ui.say(`Removed ${r.removed} downloads`);
    } catch (e: any) {
      ui.say(`Could not remove: ${e.message}`);
    }
  }

  async keep(id: string) {
    if (this.#off()) return;
    const title = catalog.collection(id)?.title;
    try {
      await call((a) => a.downloadKeep(id));
      ui.say(title ? `Downloading “${title}”. New songs in it are downloaded too.` : "Downloading");
    } catch (e: any) {
      ui.say(e.code === "authRequired" ? "Sign in to download your liked songs" : `Could not download: ${e.message}`);
    }
  }

  /** Stop keeping a collection (this copy, or the likeliest one), after
   *  saying what goes. */
  unkeep(id: string, copy?: KeptCollection) {
    const k = copy ?? this.keptFor(id);
    if (!k) return;
    ui.confirm = {
      title: "Remove this download?",
      text: `The downloaded songs of “${k.card.title}” (${songs(k.ready)}, ${sizeText(k.bytes)}) are removed from this computer. Songs you downloaded separately, or that another downloaded playlist or album includes, stay. Nothing changes on YouTube Music.`,
      confirm: "Remove download",
      danger: true,
      run: async () => {
        try {
          await call((a) => a.downloadUnkeep(k.card.id, k.account));
          ui.say(`Removed the download of “${k.card.title}”`);
        } catch (e: any) {
          ui.say(`Could not remove: ${e.message}`);
        }
      },
    };
  }

  sync(id: string | null = null) {
    call((a) => a.downloadsSync(id))
      .then(() => ui.say("Checking for changes"))
      .catch((e: BackendError) => ui.say(e.message));
  }

  pause(on: boolean) {
    call((a) => a.downloadsPause(on))
      .then(() => ui.say(on ? "Downloads paused" : "Downloads resumed"))
      .catch((e: BackendError) => ui.say(`Could not change that: ${e.message}`));
  }

  pauseKept(id: string, on: boolean, copy?: KeptCollection) {
    const k = copy ?? this.keptFor(id);
    if (!k) return;
    call((a) => a.downloadPauseCollection(k.card.id, on, k.account))
      .then(() => ui.say(on ? `Paused downloading “${k.card.title}”` : `Resumed downloading “${k.card.title}”`))
      .catch((e: BackendError) => ui.say(`Could not change that: ${e.message}`));
  }

  retry() {
    call((a) => a.downloadsRetry())
      .then(() => ui.say("Trying again"))
      .catch((e: BackendError) => ui.say(`Could not try again: ${e.message}`));
  }

  removeAll() {
    const s = this.summary;
    ui.confirm = {
      title: "Remove all downloads?",
      text: `${songs(s?.songs ?? 0)} (${sizeText(s?.bytes)}) are removed from this computer, for every account, along with downloads that are waiting or failed, and no playlist or album stays downloaded. Nothing changes on YouTube Music.`,
      confirm: "Remove all",
      danger: true,
      run: async () => {
        try {
          await call((a) => a.downloadsRemoveAll());
          ui.say("Removed all downloads");
        } catch (e: any) {
          ui.say(`Could not remove: ${e.message}`);
        }
      },
    };
  }
}

export const player = new Player();
export const nav = new Nav();
export const ui = new UI();
export const account = new Account();
export const playlists = new Playlists();
export const downloads = new Downloads();
export const updates = new Updates();

/** Why a collection can't be downloaded offline: it has to be read first. */
export const KEEP_OFFLINE = "You're offline. Encore needs to read the list first, so try again when you're back online.";

/** Offline, actions that need YouTube say why instead of running (D23). */
export const needsOnline = (what: string) => (ui.online ? undefined : `You're offline. Connect to the internet to ${what}.`);

/** One line on how a kept collection is doing, for its header and the
 *  Downloads page. */
export function keptNote(k: KeptCollection | undefined, syncing: boolean): string | null {
  if (!k) return null;
  if (!k.mine) return k.channel ? `Kept by ${k.channel}. Sign in as ${k.channel} to keep it up to date.` : "Downloaded while signed out. It isn't checked for changes while you're signed in.";
  if (k.gone) return k.error;
  if (syncing) return "Checking for changes…";
  if (k.error) return k.error;
  if (k.paused) return `Paused · ${k.ready} of ${songs(k.songs)} downloaded`;
  if (!k.songs) return "Reading the list…";
  if (k.ready + k.failed < k.songs) return `${k.ready} of ${songs(k.songs)} downloaded`;
  if (k.failed) return `${k.failed === 1 ? "1 song" : `${k.failed} songs`} couldn't be downloaded. Try again from the Downloads page.`;
  return "Downloaded · new songs are downloaded too";
}

/** The download action for a song, in the words of its state. */
function downloadItem(id: string): MenuItem {
  const st = downloads.items.get(id)?.state;
  if (st === "ready") return { label: "Remove download", icon: "downloaded", run: () => void downloads.remove([id]) };
  if (st === "queued" || st === "downloading") return { label: "Stop downloading", icon: "x", run: () => void downloads.remove([id], true) };
  if (st === "failed") return { label: "Download again", icon: "download", run: () => void downloads.download([id]) };
  return { label: "Download", icon: "download", run: () => void downloads.download([id]) };
}

const remote = (id: string) => id.replace(/^(ytm|mock):/, "");

/** Open a web page in the system browser. */
export function openExternal(url: string) {
  if (isTauri) import("@tauri-apps/plugin-opener").then((m) => m.openUrl(url)).catch(() => ui.say("Could not open the browser"));
  else window.open(url, "_blank", "noopener");
}

// ---------- song / video versions ----------

/** Known pairs by track id; `null` = looked up, none. */
export const versions = new SvelteMap<string, Counterpart | null>();
const looking = new Map<string, Promise<Counterpart | null>>();

/** The other version of a track: what it already knows, else the backend's
 *  lookup (D21). Remembered for the session. */
export function lookupVersion(t: Track): Promise<Counterpart | null> {
  if (versions.has(t.id)) return Promise.resolve(versions.get(t.id) ?? null);
  let p = looking.get(t.id);
  if (!p) {
    p = call((a) => a.counterpart(t))
      .then((c) => {
        versions.set(t.id, c);
        if (c) catalog.addTracks([c.track]);
        return c;
      })
      .catch(() => null)
      .finally(() => looking.delete(t.id));
    looking.set(t.id, p);
  }
  return p;
}

/** "Video" or "Song": what switching gets you. */
export const otherKind = (t: Track) => (t.kind === "video" ? "song" : "video");

/** Switch a queue entry (by uid) to its other version, or play the other
 *  version of a track that isn't in the queue. */
export async function playOtherVersion(t: Track, uid?: string) {
  const c = await lookupVersion(t);
  if (!c) return ui.say(t.kind === "video" ? "YouTube Music has no song version of this video" : "YouTube Music has no video version of this song");
  try {
    if (uid) await call((a) => a.switchVersion(uid));
    else await call((a) => a.playVersion(t));
  } catch (e: any) {
    ui.say(`Could not switch: ${e.message}`);
  }
}

/** Shared track actions so every menu uses the same words for the same thing. */
export function trackMenu(id: string, uid?: string): MenuItem[] {
  const tr = catalog.track(id);
  if (!tr) return [];
  // The playing entry switches in place; other rows play the other version.
  const entry = uid ?? (player.current?.id === id ? player.currentEntry?.uid : undefined);
  const liked = player.liked.has(id);
  const artist = tr.artists.find((a) => a.id);
  // Five groups (`TrackBand`): play it, keep it, this song, go to, and
  // "Don't play this again" alone at the bottom.
  const band: Record<TrackBand, MenuItem[]> = { play: [], keep: [], song: [], goto: [], last: [] };
  band.play.push(
    { label: "Play next", icon: "playNext", run: () => (player.playNext(id), ui.say(`“${tr.title}” plays next`)) },
    { label: "Add to queue", icon: "queue", run: () => (player.addToQueue(id), ui.say(`Added “${tr.title}” to queue`)) },
  );
  band.keep.push({ label: liked ? "Remove from liked music" : "Add to liked music", icon: liked ? "heartFill" : "heart", disabled: needsOnline("change your likes"), run: () => (player.toggleLike(id), ui.say(liked ? "Removed from liked music" : "Added to liked music")) });
  if (account.signedIn) band.keep.push({ label: "Add to playlist…", icon: "plus", disabled: needsOnline("add to a playlist"), run: () => void pickPlaylist([id]) });
  const extra = trackMenuItems.items({ track: tr, uid: entry });
  const keepExtra = extra.filter((i) => i.band === "keep");
  band.keep.push(...keepExtra);
  if (downloads.available && tr.playable) band.keep.push(downloadItem(id));
  for (const i of extra) if (i.band !== "keep") band[i.band ?? "song"].push(i);
  if ((tr.kind === "song" || tr.kind === "video") && versions.get(id) !== null) {
    band.song.push({ label: tr.kind === "video" ? "Play the song version" : "Play the video version", icon: "swap", disabled: needsOnline("switch versions"), run: () => void playOtherVersion(tr, entry) });
  }
  band.song.push({ label: "Start radio", icon: "radio", disabled: needsOnline("start a radio"), run: () => player.playTrack(id) });
  if (artist?.id) band.goto.push({ label: "Go to artist", icon: "artist", run: () => nav.go({ name: "artist", id: artist.id! }) });
  if (tr.album?.id) band.goto.push({ label: "Go to album", icon: "album", run: () => nav.go({ name: "collection", id: tr.album!.id! }) });
  band.goto.push({ label: "Copy link", icon: "link", run: () => (navigator.clipboard?.writeText(`https://music.youtube.com/watch?v=${remote(id)}`), ui.say("Link copied")) });
  return grouped(band.play, band.keep, band.song, band.goto, band.last);
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
  const full = c && "editable" in c ? c : null;
  const mineItems: MenuItem[] =
    full?.editable && account.signedIn && full.playlistId
      ? [
          { label: "Edit details…", icon: "edit", disabled: needsOnline("change a playlist"), run: () => editPlaylist(id) },
          { label: "Delete playlist…", icon: "trash", danger: true, disabled: needsOnline("delete a playlist"), run: () => deletePlaylist(id) },
        ]
      : [];
  // Another account's copy doesn't stop this one keeping its own (D23).
  const kept = downloads.keptFor(id);
  const dl: MenuItem[] = !downloads.available
    ? []
    : kept?.mine
      ? [{ label: "Remove download…", icon: "downloaded", run: () => downloads.unkeep(id, kept) }]
      : [{ label: "Download", icon: "download", disabled: ui.online ? undefined : KEEP_OFFLINE, run: () => void downloads.keep(id) }];
  return grouped(
    [
      { label: "Play", icon: "play", run: () => player.playCollection(id) },
      { label: "Shuffle", icon: "shuffle", run: () => player.playCollection(id, 0, true) },
      { label: "Play next", icon: "playNext", run: () => withTracks((ids) => (player.playNext(ids), ui.say(`“${title}” plays next`))) },
      { label: "Add to queue", icon: "queue", run: () => withTracks((ids) => (player.addToQueue(ids), ui.say(`Added “${title}” to queue`))) },
    ],
    [...dl, ...collectionMenuItems.items(id)],
    [
      {
        label: "Copy link",
        icon: "link",
        run: () => (navigator.clipboard?.writeText(listId.startsWith("MPRE") ? `https://music.youtube.com/browse/${listId}` : `https://music.youtube.com/playlist?list=${listId}`), ui.say("Link copied")),
      },
    ],
    mineItems,
  );
}

// ---------- playlist actions (D20) ----------

export function newPlaylist(trackIds: string[] = []) {
  if (!account.signedIn) return ui.say("Sign in to create playlists");
  ui.sheet = { mode: "create", trackIds };
}

export function editPlaylist(id: string) {
  const c = catalog.collection(id);
  if (!c || !("editable" in c) || !c.playlistId) return;
  ui.sheet = { mode: "edit", id, playlistId: c.playlistId, title: c.title, description: c.description ?? "", privacy: c.privacy ?? undefined };
}

export function deletePlaylist(id: string) {
  const c = catalog.collection(id);
  if (!c || !("editable" in c) || !c.playlistId) return;
  const pid = c.playlistId;
  ui.confirm = {
    title: "Delete this playlist?",
    text: `“${c.title}” is deleted from YouTube Music. Songs already in your queue keep playing.`,
    confirm: "Delete",
    danger: true,
    run: async () => {
      const before = playlists.list;
      playlists.remove(id);
      const r = await call((a) => a.playlistDelete(pid)).catch((e: BackendError) => (ui.say(`Could not delete: ${e.message}`), null));
      if (r === "done") {
        ui.say(`Deleted “${c.title}”`);
        if (nav.view.name === "collection" && nav.view.id === id) nav.go({ name: "library" });
      } else if (r === "outcomeUnknown") {
        ui.say("Could not confirm the delete; check your playlists");
        playlists.changed();
      } else {
        playlists.list = before;
        if (r === "rejected") ui.say("YouTube Music did not delete that playlist");
      }
    },
  };
}

/** Add songs to a playlist; a duplicate asks before adding it again. */
export async function addToPlaylist(p: { playlistId: string; title: string }, trackIds: string[], allowDuplicates = false) {
  const r: MutationOutcome | null = await call((a) => a.playlistAdd(p.playlistId, trackIds, allowDuplicates)).catch((e: BackendError) => (ui.say(`Could not add: ${e.message}`), null));
  if (r === "done") ui.say(`Added to “${p.title}”`);
  else if (r === "duplicate") {
    const one = trackIds.length === 1 ? catalog.track(trackIds[0])?.title : null;
    ui.confirm = {
      title: "Already in this playlist",
      text: one ? `“${one}” is already in “${p.title}”. Add it again?` : `Some of these songs are already in “${p.title}”. Add them again?`,
      confirm: "Add anyway",
      run: () => void addToPlaylist(p, trackIds, true),
    };
  } else if (r === "outcomeUnknown") ui.say("Not sure that worked; check the playlist");
  else if (r === "rejected") ui.say("YouTube Music did not accept that");
}

/** Second menu: "New playlist…" and your playlists, to add songs to. */
async function pickPlaylist(trackIds: string[]) {
  if (!playlists.loaded) await playlists.load();
  const lists = playlists.list.filter((i) => i.kind !== "liked");
  ui.openMenuAgain([
    { label: "New playlist…", icon: "plus", run: () => newPlaylist(trackIds) },
    ...lists.slice(0, 14).map((p) => ({
      label: p.title,
      icon: "list",
      run: () => void addToPlaylist({ playlistId: p.id.replace(/^(ytm|mock):/, "").replace(/^VL/, ""), title: p.title }, trackIds),
    })),
  ]);
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
      notice: (n) => ui.say(n.text, n.level === "error" ? "error" : n.level === "warn" ? "warn" : "info"),
      signin: (s) => (account.signin = { stage: s.stage, message: s.message }),
      playlists: (e) => playlists.changed(e.playlistId),
      history: () => ui.historyRev++,
      downloads: (s) => downloads.onSummary(s),
      downloadItem: (e) => downloads.onItem(e),
      connectivity: (on) => (ui.online = on),
      update: (s) => updates.apply(s),
    }),
  );
  await subscribeFeatures();
  const b = await call((a) => a.bootstrap());
  account.state = b.auth;
  ui.reportHistory = b.reportHistory;
  ui.online = b.online;
  if (b.notice) ui.say(b.notice, "error");
  await downloads.refresh();
  updates.apply(b.update);
  player.setQueue(b.queue.entries, b.queue.revision, b.queue.index);
  player.applySnapshot(b.player);
  // Lyrics for a restored track.
  const cur = player.current;
  if (cur && player.lyricsFor !== cur.id) {
    call((a) => a.lyrics(cur)).then((l) => player.setLyrics(cur.id, l)).catch(() => {});
  }
  void loadFeatures();
}

export { catalog };

// The feature modules (./features): their state, actions and hooks.
export * from "./features";
