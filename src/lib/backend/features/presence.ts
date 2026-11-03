// Presence (lane E): Discord Rich Presence (D31).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/presence.rs). The mock
// is ../mock/presence.ts. Reached as `call((a) => a.features.presence.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.
import { commands, events } from "../bindings";
import type { PresenceStatus } from "../types";
import { u } from "../unwrap";

export interface PresenceApi {
  status(): Promise<PresenceStatus>;
  set(on: boolean): Promise<PresenceStatus>;
  subscribe(h: (s: PresenceStatus) => void): Promise<() => void>;
}

export const tauriPresence: PresenceApi = {
  status: () => u(commands.presenceStatus()),
  set: (on) => u(commands.presenceSet(on)),
  subscribe: (h) => events.presenceEvent.listen((e) => h(e.payload)),
};
