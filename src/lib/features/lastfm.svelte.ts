// Last.fm (lane E): connecting, scrobbling and play counts (D31).
//
// This module is the area's UI state and actions (`call((a) => a.features.lastfm.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { call, type BackendError, type LastfmStatus, type Track } from "$lib/backend";
import { openExternal, ui } from "$lib/state.svelte";
import { registerFeature } from "./hooks";

/** Where Last.fm hands out API keys. */
export const LASTFM_CREATE = "https://www.last.fm/api/account/create";

class Lastfm {
  status = $state<LastfmStatus | null>(null);
  /** The connect sheet is open. */
  sheet = $state(false);
  /** Connected and sending: counts can be asked for (lane B's sort). */
  connected = $derived(!!this.status?.username && !this.status.error);

  apply(s: LastfmStatus) {
    this.status = s;
  }

  async load() {
    this.apply(await call((a) => a.features.lastfm.status()));
  }

  /** Step one: check the key and secret, then open Last.fm's approval
   *  page; returns its address. */
  async begin(key: string, secret: string): Promise<string> {
    const url = await call((a) => a.features.lastfm.begin(key, secret));
    openExternal(url);
    return url;
  }

  /** Step two, after approving in the browser. */
  async finish() {
    this.apply(await call((a) => a.features.lastfm.finish()));
    this.sheet = false;
    ui.say(`Connected to Last.fm as ${this.status?.username}`);
  }

  cancel() {
    this.sheet = false;
    call((a) => a.features.lastfm.cancel()).then((s) => this.apply(s)).catch(() => {});
  }

  disconnect() {
    const s = this.status;
    const waiting = s?.queued ? ` ${s.queued === 1 ? "1 play waiting to be sent stays" : `${s.queued} plays waiting to be sent stay`} here until you connect this account again.` : "";
    ui.confirm = {
      title: "Disconnect Last.fm?",
      text: `Encore stops scrobbling to ${s?.username ?? "Last.fm"} and forgets its key.${waiting} Your scrobbles on Last.fm stay.`,
      confirm: "Disconnect",
      danger: true,
      run: () =>
        call((a) => a.features.lastfm.disconnect())
          .then((st) => (this.apply(st), ui.say("Disconnected from Last.fm")))
          .catch((e: BackendError) => ui.say(`Could not disconnect: ${e.message}`, "error")),
    };
  }

  /** Last.fm play counts, in order (null: no match), or nulls when not connected. */
  plays(tracks: Track[]): Promise<(number | null)[]> {
    if (!this.connected) return Promise.resolve(tracks.map(() => null));
    return call((a) => a.features.lastfm.plays(tracks)).catch(() => tracks.map(() => null));
  }
}

export const lastfm = new Lastfm();

registerFeature({
  name: "lastfm",
  subscribe: () => call((a) => a.features.lastfm.subscribe((s) => lastfm.apply(s))),
  load: () => lastfm.load(),
});
