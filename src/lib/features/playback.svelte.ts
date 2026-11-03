// Playback (lane A): the A–B loop and the sleep timer (D26), shuffle cycles
// and the Autoplay rule (D33).
//
// This module is the area's UI state and actions (`call((a) => a.features.playback.…)`).
// It registers its menu items, palette commands, chips and keys with
// ./hooks at load time, and is re-exported from $lib/state.svelte. It is
// loaded while state.svelte.ts is still loading: read `player`, `ui` and
// the rest only inside functions and getters, never at the top level or in
// a field initialiser. The snapshot's reserved fields arrive as
// `player.abLoop`, `player.sleep` and `player.privateSession`.
//
// The loop and the timer are Rust's (D26): this module asks for them and
// shows what the snapshot says. Shuffle cycles need nothing here.
import { call, type BackendError, type SleepMode } from "$lib/backend";
import type { PlaybackApi } from "$lib/backend/features/playback";
import { catalog } from "$lib/catalog.svelte";
import { fmt } from "$lib/data";
import { player, ui } from "$lib/state.svelte";
import { addKey, deckChips, paletteItems, registerFeature, trackMenuItems, type MenuItem, type PaletteItem } from "./hooks";

registerFeature({ name: "playback" });

/** Ask Rust; says why if it refuses. True when it was done. */
const api = (f: (p: PlaybackApi) => Promise<unknown>) =>
  call((a) => f(a.features.playback)).then(
    () => true,
    (e: BackendError) => (ui.say(e.message, "warn"), false),
  );

// ---------- the A–B loop ----------

/** The length "Loop a part…" starts with; the ends can then be dragged. */
const FIRST_LOOP = 15;

class Loop {
  /** <kbd>A</kbd> pressed once: the loop's start, waiting for its end. */
  start = $state<{ at: number; trackId: string } | null>(null);

  /** The loop on the song playing, in seconds. */
  get range(): { a: number; b: number } | null {
    const l = player.abLoop;
    return l?.a != null && l.b != null ? { a: l.a, b: l.b } : null;
  }

  /** Where the waiting start is, on the song playing. */
  get pending(): number | null {
    const s = this.start;
    return s && s.trackId === player.current?.id && !player.abLoop ? s.at : null;
  }

  set(a: number, b: number) {
    this.start = null;
    void api((p) => p.loopSet(a, b));
  }

  clear() {
    this.start = null;
    if (player.abLoop) void api((p) => p.loopClear());
  }

  /** "Loop a part…": a loop from here, which the handles on the seek bar
   *  then adjust. */
  part() {
    const dur = player.duration ?? player.current?.duration ?? 0;
    if (!dur) return ui.say("This song's length isn't known yet. Try again in a moment.");
    const at = player.position;
    const a = at + FIRST_LOOP <= dur ? at : Math.max(0, dur - FIRST_LOOP);
    this.set(a, Math.min(dur, a + FIRST_LOOP));
    ui.say("Looping. Drag its ends on the seek bar, or press A to set them as it plays.");
  }

  /** <kbd>A</kbd>: the first press marks the start, the second the end, a
   *  third clears the loop. */
  press() {
    const cur = player.current;
    if (!cur) return;
    if (player.abLoop) {
      this.clear();
      return ui.say("Loop off");
    }
    const t = player.position;
    const p = this.pending;
    if (p == null) {
      this.start = { at: t, trackId: cur.id };
      return ui.say(`Loop from ${fmt(t)}. Press A again where it should end.`);
    }
    if (Math.abs(t - p) < 1) return ui.say("A loop needs to be at least a second long.");
    this.set(Math.min(p, t), Math.max(p, t));
  }
}

export const abLoop = new Loop();

// ---------- the sleep timer ----------

/** A clock for counting down between snapshots. */
const clock = $state({ now: Date.now() });
if (typeof window !== "undefined") setInterval(() => (clock.now = Date.now()), 1000);
/** When each snapshot's timer arrived (its `remaining` is from then). */
const arrived = new WeakMap<object, number>();

const CHOICES: [number, string][] = [
  [15, "15 minutes"],
  [30, "30 minutes"],
  [45, "45 minutes"],
  [60, "1 hour"],
];

const minutesText = (s: number) => {
  const m = Math.max(1, Math.round(s / 60));
  return m >= 60 ? `${Math.floor(m / 60)} hr${m % 60 ? ` ${m % 60} min` : ""}` : m === 1 ? "1 minute" : `${m} minutes`;
};

class SleepTimer {
  /** Seconds until a timed sleep, counting down; null for the other kinds. */
  get left(): number | null {
    const s = player.sleep;
    if (!s || s.mode !== "timer" || s.remaining == null) return null;
    let at = arrived.get(s);
    if (at == null) arrived.set(s, (at = Date.now()));
    return Math.max(0, s.remaining - (clock.now - at) / 1000);
  }

  /** What the chip and the menu say about the running timer. */
  get status(): string | null {
    const s = player.sleep;
    if (!s) return null;
    if (s.mode === "endOfSong") return "Pauses at the end of this song";
    if (s.mode === "endOfQueue") return "Pauses at the end of the queue";
    return `Pauses in ${minutesText(this.left ?? 0)}`;
  }

  set(mode: SleepMode, minutes: number | null = null) {
    void api((p) => p.sleepSet(mode, minutes)).then((ok) => {
      if (!ok) return;
      ui.say(mode === "endOfSong" ? "Pausing at the end of this song" : mode === "endOfQueue" ? "Pausing at the end of the queue" : `Pausing in ${minutesText((minutes ?? 0) * 60)}`);
    });
  }

  off() {
    void api((p) => p.sleepCancel()).then((ok) => ok && ui.say("Sleep timer off"));
  }

  /** The moon button's menu (and the chip's). */
  menu(): MenuItem[] {
    const on = player.sleep;
    const looping = !!player.abLoop;
    return [
      ...CHOICES.map<MenuItem>(([m, label]) => ({ label, icon: "clock", run: () => this.set("timer", m) })),
      { label: "End of this song", icon: "moon", disabled: looping ? "Clear the loop to stop at the end of this song" : undefined, run: () => this.set("endOfSong") },
      { label: "End of the queue", icon: "queue", run: () => this.set("endOfQueue") },
      ...(on ? [{ label: "Turn off the sleep timer", icon: "x", run: () => this.off() }] : []),
    ];
  }
}

export const sleepTimer = new SleepTimer();

// ---------- Autoplay (D33) ----------

/** The line under an "Autoplay" switch, in the queue panel and in Settings. */
export const autoplayCopy = (on: boolean, online = true) =>
  !online ? "Waits until you're back online" : on ? "Keep playing similar songs when the queue ends" : "Off: only what you choose plays";

/** Play an artist: their radio if YouTube offers one, else the top songs.
 *  With Autoplay off nothing is chosen for the user (D33): Play and
 *  Shuffle play the artist's own songs, the whole list when there is one. */
export async function playArtist(id: string, shuffle = false) {
  const a = await call((x) => x.artist(id)).catch((e: BackendError) => (ui.say(`Could not load the artist: ${e.message}`), null));
  if (!a) return;
  catalog.addArtist(a);
  const own = () => player.playTracks(a.topSongs, 0, { label: a.name, collectionId: null, radioPlaylist: null }, shuffle);
  if (!player.autoplay) {
    const all = a.topSongsMore;
    if (all) player.playCollection(`ytm:${all.browseId}`, 0, shuffle, all.params ?? null);
    else if (a.topSongs.length) own();
    else ui.say("This artist has no songs to play. Turn Autoplay on to play their radio.");
    return;
  }
  const radio = shuffle ? a.shufflePlaylist ?? a.radioPlaylist : a.radioPlaylist ?? a.shufflePlaylist;
  if (radio) player.startRadio(radio, a.name);
  else own();
}

// ---------- hooks ----------

trackMenuItems.add(({ track }) => {
  if (track.id !== player.current?.id) return [];
  return player.abLoop
    ? [{ label: "Clear the loop", icon: "x", run: () => abLoop.clear() }]
    : [{ label: "Loop a part…", icon: "repeat", run: () => abLoop.part() }];
});

deckChips.add(() => {
  const s = player.sleep;
  if (!s) return [];
  const left = sleepTimer.left;
  const label = s.mode === "timer" ? fmt(left) : s.mode === "endOfSong" ? "End of song" : "End of queue";
  return [{ key: "sleep", icon: "moon", label, title: `Sleep timer: ${sleepTimer.status?.toLowerCase()}. Change or turn it off.`, run: () => ui.openMenuAgain(sleepTimer.menu()) }];
});

paletteItems.add((q) => {
  const out: PaletteItem[] = [];
  const sub = "Sleep timer";
  if (player.sleep) out.push({ key: "sleep-off", group: "Actions", icon: "moon", title: "Turn off the sleep timer", sub: sleepTimer.status ?? sub, run: () => sleepTimer.off() });
  if (player.abLoop) out.push({ key: "loop-off", group: "Actions", icon: "repeat", title: "Clear the loop", sub: `${fmt(player.abLoop.a)}–${fmt(player.abLoop.b)}`, hint: "A", run: () => abLoop.clear() });
  // The rest only when asked for: the idle list stays short.
  if (!q) return out;
  if (player.current && !player.abLoop) out.push({ key: "loop", group: "Actions", icon: "repeat", title: "Loop a part of this song", sub: "A–B loop", hint: "A", run: () => abLoop.part() });
  for (const [m, label] of CHOICES) out.push({ key: `sleep-${m}`, group: "Actions", icon: "moon", title: `Sleep in ${label}`, sub, run: () => sleepTimer.set("timer", m) });
  if (player.current && !player.abLoop) out.push({ key: "sleep-song", group: "Actions", icon: "moon", title: "Sleep at the end of this song", sub, run: () => sleepTimer.set("endOfSong") });
  out.push({ key: "sleep-queue", group: "Actions", icon: "moon", title: "Sleep at the end of the queue", sub, run: () => sleepTimer.set("endOfQueue") });
  return out;
});

addKey({ key: "a", label: ["A", "Loop: set its start, then its end; again to clear"], run: () => abLoop.press() });
