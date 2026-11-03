// Every area's mock, as `api.features` in `bun run dev`: one dispatch per
// feature module.
import { mockPlayback } from "./playback";
import { mockLibraryTools } from "./libraryTools";
import { mockSaved } from "./saved";
import { mockTaste } from "./taste";
import { mockStats } from "./stats";
import { mockLastfm } from "./lastfm";
import { mockPresence } from "./presence";
import { mockWiki } from "./wiki";
import { mockDetails } from "./details";
import type { FeatureApis } from "../features/types";

export const mockFeatures: FeatureApis = {
  playback: mockPlayback,
  libraryTools: mockLibraryTools,
  saved: mockSaved,
  taste: mockTaste,
  stats: mockStats,
  lastfm: mockLastfm,
  presence: mockPresence,
  wiki: mockWiki,
  details: mockDetails,
};
