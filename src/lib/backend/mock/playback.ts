// The mock of playback (lane A) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts; the loop and the sleep timer run in its simulated player
// (`sim*` there), like the Rust player's.
//
// `?loop` starts with a loop on the song that's loaded, `?sleep` with a
// sleep timer 23 minutes from its end; `?autoplayoff` starts with Autoplay
// off (try an artist's Shuffle: their songs, not a radio).
import type { PlaybackApi } from "../features/playback";
import type * as T from "../types";
import { simCancelSleep, simClearLoop, simLoop, simSleep } from "../mock";

const sleep = { mode: null as T.SleepMode | null, deadline: 0 };

/** The snapshot's reserved fields (D26, D36), read by ../mock.ts. A private
 *  session is lane D's to start; it sets `privateUntil` here. */
export const mockPlayer = {
  loop: null as T.LoopRange | null,
  /** Repeat-one, set aside while a loop runs (D26). */
  loopRepeat: null as T.Repeat | null,
  get sleep(): T.SleepTimer | null {
    if (!sleep.mode) return null;
    return { mode: sleep.mode, remaining: sleep.mode === "timer" ? Math.max(0, (sleep.deadline - Date.now()) / 1000) : null };
  },
  /** The timer's raw state, for the simulated player. */
  sleepState: sleep,
  /** Unix ms a private session ends at (lane D starts it), like the player's clock (D36). */
  privateUntil: null as number | null,
  get private(): T.PrivateSession | null {
    return this.privateUntil == null ? null : { remaining: Math.max(0, (this.privateUntil - Date.now()) / 1000) };
  },
};

export const mockPlayback: PlaybackApi = {
  async loopSet(a, b) {
    simLoop(a, b);
    return null;
  },
  async loopClear() {
    simClearLoop();
    return null;
  },
  async sleepSet(mode, minutes) {
    simSleep(mode, minutes);
    return null;
  },
  async sleepCancel() {
    simCancelSleep();
    return null;
  },
};
