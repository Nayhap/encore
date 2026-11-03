// Presence (lane E): Discord Rich Presence (D31).
//
// This module is the area's UI state and actions (`call((a) => a.features.presence.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { call, type BackendError, type PresenceStatus } from "$lib/backend";
import { ui } from "$lib/state.svelte";
import { registerFeature } from "./hooks";

class Presence {
  status = $state<PresenceStatus | null>(null);

  async set(on: boolean) {
    const before = this.status;
    if (before) this.status = { ...before, enabled: on, connected: false };
    try {
      this.status = await call((a) => a.features.presence.set(on));
    } catch (e) {
      this.status = before;
      ui.say(`Could not change that: ${(e as BackendError).message}`);
    }
  }
}

export const presence = new Presence();

registerFeature({
  name: "presence",
  subscribe: () => call((a) => a.features.presence.subscribe((s) => (presence.status = s))),
  load: async () => (presence.status = await call((a) => a.features.presence.status())),
});
