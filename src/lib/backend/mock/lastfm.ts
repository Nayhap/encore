// The mock of Last.fm (lane E) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
import type { LastfmApi } from "../features/lastfm";

export const mockLastfm: LastfmApi = {};
