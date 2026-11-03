// The mock of presence (lane E) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
import type { PresenceApi } from "../features/presence";

export const mockPresence: PresenceApi = {};
