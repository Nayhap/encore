// The mock of lastfm (lane E) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts. `?lastfm` starts connected (with plays waiting when
// `?offline`); `?lastfmexpired` starts with a session Last.fm refuses.
import type { LastfmApi } from "../features/lastfm";
import { mockKit } from "../mock";
import type * as T from "../types";

const q = new URLSearchParams(location.search);
const st: T.LastfmStatus = {
  username: q.has("lastfm") || q.has("lastfmexpired") ? "nightowl_rj" : null,
  approving: false,
  queued: q.has("lastfm") && q.has("offline") ? 3 : 0,
  failed: 0,
  error: q.has("lastfmexpired") ? "Last.fm no longer accepts Encore's connection. Connect again to send your plays." : null,
};
let listener: ((s: T.LastfmStatus) => void) | null = null;
const emit = () => listener?.({ ...st });
const fail = (code: string, message: string) => ({ code, message, retryable: false, action: null });
const hex = /^[0-9a-f]{32}$/i;
const hash = (s: string) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);

export const mockLastfm: LastfmApi = {
  status: () => mockKit.delay({ ...st }),
  async begin(key, secret) {
    if (!hex.test(key.trim())) throw fail("invalidArgument", "That isn't a Last.fm API key: it has 32 letters and digits.");
    if (!hex.test(secret.trim())) throw fail("invalidArgument", "That isn't a Last.fm shared secret: it has 32 letters and digits.");
    if (!mockKit.online) throw fail("network", "You're offline. Connect to the internet to connect Last.fm.");
    st.approving = true;
    emit();
    return mockKit.delay(`https://www.last.fm/api/auth/?api_key=${key.trim()}&token=mock`, 500);
  },
  async finish() {
    await mockKit.delay(null, 700);
    Object.assign(st, { username: "nightowl_rj", approving: false, error: null });
    emit();
    return { ...st };
  },
  async cancel() {
    st.approving = false;
    emit();
    return { ...st };
  },
  async disconnect() {
    Object.assign(st, { username: null, approving: false, error: null, queued: 0, failed: 0 });
    emit();
    return mockKit.delay({ ...st });
  },
  plays: (tracks) => mockKit.delay(tracks.map((t) => (st.username && !st.error && hash(t.id) % 4 ? hash(t.id + "fm") % 180 : null)), 300),
  async subscribe(h) {
    listener = h;
    return () => (listener = null);
  },
};
