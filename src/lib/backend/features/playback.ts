// Playback (lane A): the A–B loop and the sleep timer (D26), shuffle cycles and the Autoplay rule (D33).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/playback.rs). The mock
// is ../mock/playback.ts. Reached as `call((a) => a.features.playback.…)`.
// The loop and the timer come back in the player snapshot (`loop`, `sleep`);
// shuffle cycles and the Autoplay rule need no calls of their own.
import { commands } from "../bindings";
import type { SleepMode } from "../types";
import { u } from "../unwrap";

export interface PlaybackApi {
  /** Loop the song playing between `a` and `b` seconds (either order). */
  loopSet(a: number, b: number): Promise<null>;
  loopClear(): Promise<null>;
  /** `minutes` for "timer"; the end of the song or of the queue otherwise. */
  sleepSet(mode: SleepMode, minutes: number | null): Promise<null>;
  sleepCancel(): Promise<null>;
}

export const tauriPlayback: PlaybackApi = {
  loopSet: (a, b) => u(commands.loopSet(a, b)),
  loopClear: () => u(commands.loopClear()),
  sleepSet: (mode, minutes) => u(commands.sleepSet(mode, minutes)),
  sleepCancel: () => u(commands.sleepCancel()),
};
