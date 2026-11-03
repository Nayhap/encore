// The mock of presence (lane E) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts. `?nodiscord` is a build without a Discord application id
// (the row is hidden); `?discordoff` has Discord not running.
import type { PresenceApi } from "../features/presence";
import { mockKit } from "../mock";
import type * as T from "../types";

const q = new URLSearchParams(location.search);
const st: T.PresenceStatus = { available: !q.has("nodiscord"), enabled: q.has("discord"), connected: q.has("discord") && !q.has("discordoff") };
let listener: ((s: T.PresenceStatus) => void) | null = null;

export const mockPresence: PresenceApi = {
  status: () => mockKit.delay({ ...st }),
  async set(on) {
    if (!st.available) throw { code: "invalidArgument", message: "This build of Encore can't show your music on Discord.", retryable: false, action: null };
    st.enabled = on;
    st.connected = false;
    // Discord answers a moment later, if it's running.
    if (on && !q.has("discordoff")) setTimeout(() => (st.connected = st.enabled) && listener?.({ ...st }), 900);
    return mockKit.delay({ ...st });
  },
  async subscribe(h) {
    listener = h;
    return () => (listener = null);
  },
};
