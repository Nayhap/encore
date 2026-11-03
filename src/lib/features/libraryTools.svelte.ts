// Library tools (lane B): find, sort, select and mass actions, duplicates (D34), the discography and community playlists (D35).
//
// This module is the area's UI state and actions (`call((a) => a.features.libraryTools.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
//
// What lives here is shared by every list: release years (one lookup at a
// time, D34), a like or unlike of many songs (one job at a time, shown as
// a chip in the player bar), and the "Add to playlists" sheet.
import { SvelteMap } from "svelte/reactivity";
import { call, type LibraryLikeEvent, type LibraryYearEvent } from "$lib/backend";
import { deckChips, registerFeature } from "./hooks";
import { player, ui } from "$lib/state.svelte";

const songs = (n: number) => (n === 1 ? "1 song" : `${n.toLocaleString()} songs`);

// ---------- release years ----------

/** Album id → year (`null`: the album gives none), as far as known. */
export const albumYears = new SvelteMap<string, number | null>();

/** The running lookup: albums read so far of how many. */
export const yearLookup = $state<{ job: number; done: number; total: number; skipped: number; stopped: boolean }>({ job: 0, done: 0, total: 0, skipped: 0, stopped: false });

/** Find the years of these albums: cached ones at once, the rest in the
 *  background, one album page at a time (D34). */
export async function lookUpYears(albumIds: string[]) {
  const want = [...new Set(albumIds)].filter((id) => !albumYears.has(id));
  if (!want.length) return;
  const r = await call((a) => a.features.libraryTools.years(want)).catch(() => null);
  if (!r) return;
  for (const [id, y] of Object.entries(r.known)) albumYears.set(id, y ?? null);
  Object.assign(yearLookup, { job: r.job, done: 0, total: r.pending, skipped: r.skipped, stopped: false });
}

export function stopYears() {
  if (yearLookup.total && yearLookup.done < yearLookup.total) void call((a) => a.features.libraryTools.yearsStop()).catch(() => {});
  Object.assign(yearLookup, { total: 0, done: 0, skipped: 0 });
}

function onYear(e: LibraryYearEvent) {
  if (e.albumId) albumYears.set(e.albumId, e.year ?? null);
  if (e.job !== yearLookup.job) return;
  yearLookup.done = e.done;
  if (e.finished) {
    yearLookup.stopped = e.stopped;
    yearLookup.total = e.stopped ? e.total : 0;
  }
}

// ---------- likes of many songs ----------

/** The running like or unlike of a selection. */
export const likeJob = $state<{ job: number; liked: boolean; done: number; total: number }>({ job: 0, liked: true, done: 0, total: 0 });
const likeRunning = () => likeJob.total > 0;

/** Like or unlike songs, one request each, paced behind playback (D34).
 *  Songs already in that state are left out. */
export async function likeMany(ids: string[], liked: boolean) {
  if (likeRunning()) return ui.say("Encore is still changing likes. Wait for it to finish, or stop it from the player bar.");
  const todo = [...new Set(ids)].filter((id) => player.liked.has(id) !== liked);
  if (!todo.length) return ui.say(liked ? "These songs are already in your liked music" : "None of these songs are in your liked music");
  const go = async () => {
    const job = await call((a) => a.features.libraryTools.like(todo, liked)).catch((e) => (ui.say(e.code === "authRequired" ? "Sign in to like songs" : `Could not change likes: ${e.message}`), null));
    if (job == null) return;
    Object.assign(likeJob, { job, liked, done: 0, total: todo.length });
    ui.say(`${liked ? "Liking" : "Unliking"} ${songs(todo.length)}, a few seconds apart`);
  };
  // Long jobs say how long they take before they start.
  if (todo.length > 100) {
    const mins = Math.ceil((todo.length * 2.6) / 60);
    ui.confirm = {
      title: liked ? `Like ${songs(todo.length)}?` : `Unlike ${songs(todo.length)}?`,
      text: `Encore changes them one at a time so YouTube Music doesn't take it for a bot. This takes about ${mins} minutes; you can keep listening.`,
      confirm: liked ? "Like them" : "Unlike them",
      run: () => void go(),
    };
  } else await go();
}

export function stopLikes() {
  if (likeJob.total) void call((a) => a.features.libraryTools.likeStop(likeJob.job)).catch(() => {});
}

function onLike(e: LibraryLikeEvent) {
  if (e.job !== likeJob.job) return;
  if (e.trackId && e.outcome === "done") {
    if (likeJob.liked) player.liked.add(e.trackId);
    else player.liked.delete(e.trackId);
  }
  likeJob.done = e.done;
  if (e.finished) {
    const { liked, done, total } = likeJob;
    likeJob.total = 0;
    const verb = liked ? "Liked" : "Unliked";
    if (e.error) ui.say(`${verb} ${done} of ${songs(total)}. ${e.error}`, "warn");
    else if (done < total) ui.say(`Stopped: ${verb.toLowerCase()} ${done} of ${songs(total)}`);
    else ui.say(`${verb} ${songs(done)}`);
  }
}

deckChips.add(() =>
  likeRunning()
    ? [{ key: "likes", icon: likeJob.liked ? "heartFill" : "heart", label: `${likeJob.done}/${likeJob.total}`, title: `${likeJob.liked ? "Liking" : "Unliking"} ${likeJob.done} of ${songs(likeJob.total)}. Click to stop.`, run: stopLikes }]
    : [],
);

// ---------- add to playlists ----------

/** The "Add to playlists" sheet: these songs, to any of your playlists
 *  except the one they're from. */
export const playlistPicker = $state<{ open: boolean; trackIds: string[]; exclude: string | null }>({ open: false, trackIds: [], exclude: null });

export function pickPlaylists(trackIds: string[], exclude: string | null = null) {
  if (!trackIds.length) return;
  Object.assign(playlistPicker, { open: true, trackIds, exclude });
}

registerFeature({
  name: "libraryTools",
  subscribe: () => call((a) => a.features.libraryTools.subscribe({ year: onYear, like: onLike })),
});
