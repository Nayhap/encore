// Playback (lane A): the A–B loop and the sleep timer (D26), shuffle cycles
// and the Autoplay rule (D33).
//
// This module is the area's UI state and actions (`call((a) => a.features.playback.…)`).
// It registers its menu items, palette commands, chips and keys with
// ./hooks at load time, and is re-exported from $lib/state.svelte. It is
// loaded while state.svelte.ts is still loading: read `player`, `ui` and
// the rest only inside functions and getters, never at the top level or in
// a field initialiser. The snapshot's reserved fields arrive as
// `player.abLoop`, `player.sleep` and `player.privateSession`.
import { call, type BackendError } from "$lib/backend";
import { catalog } from "$lib/catalog.svelte";
import { player, ui } from "$lib/state.svelte";
import { registerFeature } from "./hooks";

registerFeature({ name: "playback" });

/** Play an artist: their radio if YouTube offers one, else the top songs.
 *  (Lane A owns it: with Autoplay off, D33 has Shuffle play a finite list.) */
export async function playArtist(id: string, shuffle = false) {
  const a = await call((x) => x.artist(id)).catch((e: BackendError) => (ui.say(`Could not load the artist: ${e.message}`), null));
  if (!a) return;
  catalog.addArtist(a);
  const radio = shuffle ? a.shufflePlaylist ?? a.radioPlaylist : a.radioPlaylist ?? a.shufflePlaylist;
  if (radio) player.startRadio(radio, a.name);
  else player.playTracks(a.topSongs, 0, { label: a.name, collectionId: null, radioPlaylist: null }, shuffle);
}
