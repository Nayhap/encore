// Taste (lane D): the block list (D36), "Hide podcasts" (D27), private sessions (D36).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/taste.rs). The mock
// is ../mock/taste.ts. Reached as `call((a) => a.features.taste.…)`.

import { commands, events } from "../bindings";
import type { BlockKind, ItemId, TasteState } from "../types";
import { u } from "../unwrap";

export interface TasteApi {
  state(): Promise<TasteState>;
  /** Block a song or an artist; a song's "Not interested" follows after the undo window. */
  block(kind: BlockKind, target: ItemId, label: string, detail: string | null): Promise<TasteState>;
  unblock(kind: BlockKind, target: ItemId): Promise<TasteState>;
  setHidePodcasts(on: boolean): Promise<TasteState>;
  /** Start (an hour) or end a private session. */
  setPrivate(on: boolean): Promise<TasteState>;
  subscribe(h: (s: TasteState) => void): Promise<() => void>;
}

export const tauriTaste: TasteApi = {
  state: () => u(commands.tasteState()),
  block: (kind, target, label, detail) => u(commands.tasteBlock(kind, target, label, detail)),
  unblock: (kind, target) => u(commands.tasteUnblock(kind, target)),
  setHidePodcasts: (on) => u(commands.tasteSetHidePodcasts(on)),
  setPrivate: (on) => u(commands.tastePrivate(on)),
  subscribe: (h) => events.tasteEvent.listen((e) => h(e.payload as TasteState)),
};
