// Stats (lane D): the Stats tab over the local play log (D30).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/stats.rs). The mock
// is ../mock/stats.ts. Reached as `call((a) => a.features.stats.…)`.

import { commands, events } from "../bindings";
import type { ItemId, ListeningStats, StatsOverview, StatsPeriod } from "../types";
import { u } from "../unwrap";

export interface StatsApi {
  overview(): Promise<StatsOverview>;
  period(p: StatsPeriod): Promise<ListeningStats>;
  /** Counted plays per song id, for "your plays" sorting; songs never counted are left out. */
  playCounts(ids: ItemId[]): Promise<Record<string, number>>;
  /** Forget the current account's plays; returns how many. */
  clear(): Promise<number>;
  /** A play was counted or ended. */
  subscribe(h: () => void): Promise<() => void>;
}

export const tauriStats: StatsApi = {
  overview: () => u(commands.statsOverview()),
  period: (p) => u(commands.statsPeriod(p)),
  playCounts: async (ids) => (await u(commands.statsPlayCounts(ids))) as Record<string, number>,
  clear: () => u(commands.statsClear()),
  subscribe: (h) => events.statsEvent.listen(() => h()),
};
