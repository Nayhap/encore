// Details (lane E): the song details sheet, tempo and key (D28).
//
// This module is the area's UI state and actions (`call((a) => a.features.details.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { call, type AudioAnalysis, type BackendError, type SongDetails, type Track } from "$lib/backend";
import { player } from "$lib/state.svelte";
import { paletteItems, registerFeature, trackMenuItems } from "./hooks";

class Details {
  /** The song the sheet is about; `null` when closed. */
  track = $state<Track | null>(null);
  info = $state<SongDetails | null>(null);
  /** The album's release year, from the album. */
  year = $state<number | null>(null);
  estimating = $state(false);
  error = $state<string | null>(null);
  #seq = 0;

  open(t: Track) {
    const my = ++this.#seq;
    this.track = t;
    this.info = null;
    this.year = null;
    this.error = null;
    this.estimating = false;
    call((a) => a.features.details.track(t))
      .then((d) => my === this.#seq && (this.info = d))
      .catch((e: BackendError) => my === this.#seq && (this.error = e.message));
    const album = t.album?.id;
    if (album && t.kind === "song") {
      call((a) => a.collection(album))
        .then((c) => my === this.#seq && (this.year = c.year ?? null))
        .catch(() => {});
    }
  }

  close() {
    if (this.estimating) this.cancel();
    this.#seq++;
    this.track = null;
  }

  async estimate() {
    const t = this.track;
    if (!t || this.estimating) return;
    const my = this.#seq;
    this.estimating = true;
    this.error = null;
    try {
      const a: AudioAnalysis = await call((x) => x.features.details.analyse(t.id));
      if (my === this.#seq && this.info) this.info = { ...this.info, analysis: a, canAnalyse: false };
    } catch (e) {
      const err = e as BackendError;
      if (my === this.#seq && err.code !== "stale") this.error = err.message;
    } finally {
      if (my === this.#seq) this.estimating = false;
    }
  }

  cancel() {
    call((a) => a.features.details.cancel()).catch(() => {});
    this.estimating = false;
  }
}

export const songDetails = new Details();

trackMenuItems.add(({ track }) => [{ label: "Song details", icon: "list", run: () => songDetails.open(track) }], 80);
paletteItems.add(() => {
  const t = player.current;
  return t ? [{ key: "e-details", group: "Actions", icon: "list", title: "Song details", sub: `${t.title}: tempo, key, plays`, run: () => songDetails.open(t) }] : [];
});

registerFeature({ name: "details" });
