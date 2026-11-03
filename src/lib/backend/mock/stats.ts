// The mock of stats (lane D) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
import type { StatsApi } from "../features/stats";

export const mockStats: StatsApi = {};
