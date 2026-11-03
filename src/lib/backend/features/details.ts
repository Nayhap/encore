// Details (lane E): the song details sheet, tempo and key (D28).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/details.rs). The mock
// is ../mock/details.ts. Reached as `call((a) => a.features.details.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.
import { commands } from "../bindings";
import type { AudioAnalysis, SongDetails, Track } from "../types";
import { u } from "../unwrap";

export interface DetailsApi {
  track(t: Track): Promise<SongDetails>;
  /** Estimate tempo and key from the song's file. */
  analyse(trackId: string): Promise<AudioAnalysis>;
  cancel(): Promise<void>;
}

export const tauriDetails: DetailsApi = {
  track: (t) => u(commands.detailsTrack(t as any)),
  analyse: (id) => u(commands.detailsAnalyse(id)),
  cancel: () => u(commands.detailsCancel()) as Promise<any>,
};
