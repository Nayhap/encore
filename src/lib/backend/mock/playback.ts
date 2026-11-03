// The mock of playback (lane A) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
import type { PlaybackApi } from "../features/playback";
import type * as T from "../types";

/** The snapshot's reserved fields (D26, D36), read by ../mock.ts. A private
 *  session is lane D's to start; it sets `private` here. */
export const mockPlayer = {
  loop: null as T.LoopRange | null,
  sleep: null as T.SleepTimer | null,
  private: null as T.PrivateSession | null,
};

export const mockPlayback: PlaybackApi = {};
