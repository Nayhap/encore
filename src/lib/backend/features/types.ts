// The feature areas' backend interfaces, under `api.features`. Each lane
// edits only its own `<area>.ts` (and `../mock/<area>.ts`), never this file.
import type { PlaybackApi } from "./playback";
import type { LibraryToolsApi } from "./libraryTools";
import type { SavedApi } from "./saved";
import type { TasteApi } from "./taste";
import type { StatsApi } from "./stats";
import type { LastfmApi } from "./lastfm";
import type { PresenceApi } from "./presence";
import type { WikiApi } from "./wiki";
import type { DetailsApi } from "./details";

export interface FeatureApis {
  /** Lane A. */
  playback: PlaybackApi;
  /** Lane B. */
  libraryTools: LibraryToolsApi;
  /** Lane C. */
  saved: SavedApi;
  /** Lane D. */
  taste: TasteApi;
  /** Lane D. */
  stats: StatsApi;
  /** Lane E. */
  lastfm: LastfmApi;
  /** Lane E. */
  presence: PresenceApi;
  /** Lane E. */
  wiki: WikiApi;
  /** Lane E. */
  details: DetailsApi;
}
