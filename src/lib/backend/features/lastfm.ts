// Last.fm (lane E): connecting, scrobbling and play counts (D31).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/lastfm.rs). The mock
// is ../mock/lastfm.ts. Reached as `call((a) => a.features.lastfm.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.
import { commands, events } from "../bindings";
import type { LastfmStatus, Track } from "../types";
import { u } from "../unwrap";

export interface LastfmApi {
  status(): Promise<LastfmStatus>;
  /** Check the key and secret; returns the page where the user approves Encore. */
  begin(apiKey: string, secret: string): Promise<string>;
  /** After the approval: keep the session. */
  finish(): Promise<LastfmStatus>;
  cancel(): Promise<LastfmStatus>;
  disconnect(): Promise<LastfmStatus>;
  /** Last.fm play counts for songs, in order (null: not matched). */
  plays(tracks: Track[]): Promise<(number | null)[]>;
  subscribe(h: (s: LastfmStatus) => void): Promise<() => void>;
}

export const tauriLastfm: LastfmApi = {
  status: () => u(commands.lastfmStatus()),
  begin: (k, s) => u(commands.lastfmBegin(k, s)),
  finish: () => u(commands.lastfmFinish()),
  cancel: () => u(commands.lastfmCancel()),
  disconnect: () => u(commands.lastfmDisconnect()),
  plays: (t) => u(commands.lastfmPlays(t as any)),
  subscribe: (h) => events.lastfmEvent.listen((e) => h(e.payload as LastfmStatus)),
};
