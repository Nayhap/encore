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
import { call, type BackendError, type LibraryLikeEvent, type LibraryYearEvent, type Privacy } from "$lib/backend";
import { catalog } from "$lib/catalog.svelte";
import { addAll } from "./copyList";
import { collectionMenuItems, deckChips, registerFeature } from "./hooks";
import { account, nav, needsOnline, player, playlists, ui } from "$lib/state.svelte";

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

// ---------- save a list as a playlist ----------

/** The running copy of a list into a new playlist. */
export const copyJob = $state<{ running: boolean; stop: boolean; title: string; done: number; total: number }>({ running: false, stop: false, title: "", done: 0, total: 0 });

/** Songs per add request, and the pause between requests (as D34's adds). */
const COPY_CHUNK = 200;
const COPY_PAUSE = 600;
const COPY_MOST = 20000;

/** Open the "New playlist" sheet for a copy of `id` (Liked Music, an
 *  album, someone's playlist). YouTube has no such thing for Liked Music,
 *  which is an automatic list. */
export function saveAsPlaylist(id: string) {
  if (!account.signedIn) return ui.say("Sign in to create playlists");
  if (copyJob.running) return ui.say(`Encore is still saving “${copyJob.title}”. Wait for it to finish, or stop it from the player bar.`);
  const c = catalog.collection(id);
  ui.sheet = { mode: "create", copyOf: id, title: c?.title ?? "", sourceTitle: c?.title ?? "this list" };
}

/** Read the whole list, make the playlist, then add its songs in order, a
 *  few hundred at a time. Nothing is created unless the list was read to
 *  its end. */
export async function copyToPlaylist(src: string, title: string, description: string, privacy: Privacy) {
  if (copyJob.running) return;
  Object.assign(copyJob, { running: true, stop: false, title, done: 0, total: 0 });
  const stopped = () => copyJob.stop;
  try {
    ui.say(`Reading the songs for “${title}”…`);
    const first = await call((a) => a.collection(src));
    const ids = first.entries.map((e) => e.track.id);
    let cursor = first.cursor ?? null;
    let whole = true;
    while (cursor && !stopped() && ids.length < COPY_MOST) {
      const p = await call((a) => a.collectionMore(cursor!));
      ids.push(...p.entries.map((e) => e.track.id));
      cursor = p.cursor ?? null;
      whole = !!cursor || p.complete;
      copyJob.total = ids.length;
    }
    if (stopped()) return ui.say(`Stopped. “${title}” wasn't created.`);
    if (!whole) return ui.say("YouTube Music stopped listing the songs partway, so no playlist was created. Try again in a moment.", "warn");
    const todo = [...new Set(ids)];
    if (!todo.length) return ui.say("There are no songs to save.");
    copyJob.total = todo.length;

    const made = await call((a) => a.playlistCreate(title, description, privacy, []));
    if (made.outcome === "outcomeUnknown") {
      playlists.changed();
      return ui.say("Could not confirm the playlist was created. Check your playlists before trying again.", "warn");
    }
    if (made.outcome !== "done" || !made.id || !made.playlistId) return ui.say("YouTube Music didn't create the playlist. Try again in a moment.", "warn");
    const newId = made.id;
    const pid = made.playlistId;
    playlists.insert({ id: newId, kind: "playlist", title, subtitle: "Playlist", artists: [], year: null, art: null, playId: null });

    let sent = 0;
    const r = await addAll(
      todo,
      async (chunk) => {
        if (sent++) await new Promise((f) => setTimeout(f, COPY_PAUSE));
        // The list has each song once already, so no duplicate check.
        // A request YouTube answers with an error (too long, or a song
        // it won't take) is a refusal of that request, tried in halves;
        // being signed out, challenged or stale stops the copy.
        const o = await call((a) => a.playlistAdd(pid, chunk, true)).catch((e: BackendError) => {
          if (["authRequired", "sessionExpired", "challenged", "stale", "network", "outcomeUnknown"].includes(e.code)) throw e;
          return "rejected" as const;
        });
        return o === "done" ? "done" : o === "outcomeUnknown" ? "unknown" : "rejected";
      },
      { size: COPY_CHUNK, stopped, progress: (n) => (copyJob.done = n) },
    );
    playlists.changed();
    const open = { label: "Open", run: () => nav.go({ name: "collection", id: newId }) };
    const of = `${r.added.toLocaleString()} of ${songs(todo.length)}`;
    if (r.unknown) ui.say(`Saved ${of} to “${title}”, then couldn't confirm a request. Check the playlist before adding the rest.`, "warn", open);
    else if (r.stopped) ui.say(`Stopped: saved ${of} to “${title}”`, "info", open);
    else if (r.added + r.refused.length < todo.length) ui.say(`Saved ${of} to “${title}”. YouTube Music refused the rest.`, "warn", open);
    else if (r.refused.length) ui.say(`Saved ${of} to “${title}”. YouTube Music refused ${songs(r.refused.length)} (removed or unavailable).`, "info", open);
    else ui.say(`Saved ${songs(r.added)} to “${title}”`, "info", open);
  } catch (e) {
    const done = copyJob.done ? ` ${copyJob.done.toLocaleString()} of ${songs(copyJob.total)} were saved.` : "";
    ui.say(`Could not save “${title}”: ${(e as BackendError).message ?? e}.${done}`, "error");
    if (copyJob.done) playlists.changed();
  } finally {
    copyJob.running = false;
  }
}

deckChips.add(() =>
  copyJob.running
    ? [{ key: "copy", icon: "list", label: copyJob.done ? `${copyJob.done}/${copyJob.total}` : copyJob.total ? `${copyJob.total}…` : "…", title: `Saving “${copyJob.title}” as a playlist${copyJob.total ? `: ${copyJob.done} of ${songs(copyJob.total)}` : ""}. Click to stop.`, run: () => (copyJob.stop = true) }]
    : [],
);

collectionMenuItems.add((id) => (account.signedIn ? [{ label: "Save as playlist…", icon: "plus", disabled: needsOnline("create a playlist"), run: () => saveAsPlaylist(id) }] : []), 30);

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
