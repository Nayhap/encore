// Every area's Tauri side, as `api.features` (loaded only under Tauri).
import { tauriPlayback } from "./playback";
import { tauriLibraryTools } from "./libraryTools";
import { tauriSaved } from "./saved";
import { tauriTaste } from "./taste";
import { tauriStats } from "./stats";
import { tauriLastfm } from "./lastfm";
import { tauriPresence } from "./presence";
import { tauriWiki } from "./wiki";
import { tauriDetails } from "./details";
import type { FeatureApis } from "./types";

export const tauriFeatures: FeatureApis = {
  playback: tauriPlayback,
  libraryTools: tauriLibraryTools,
  saved: tauriSaved,
  taste: tauriTaste,
  stats: tauriStats,
  lastfm: tauriLastfm,
  presence: tauriPresence,
  wiki: tauriWiki,
  details: tauriDetails,
};
