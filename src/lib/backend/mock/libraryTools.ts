// The mock of library tools (lane B) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
//
// Every third song has a tempo, songs in the first albums have plays in
// Encore, and `?lastfm` adds Last.fm counts, so each sort shows up. Years
// arrive 0.4 s apart, likes 0.5 s apart, as if paced.
import type { LibraryToolsApi, LibraryToolsHandlers } from "../features/libraryTools";
import type * as T from "../types";

let h: LibraryToolsHandlers | null = null;
const kit = async () => await import("../mock");
const hash = (s: string) => [...s].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
const lastfm = new URLSearchParams(location.search).has("lastfm");

let yearsJob = 0;
let likeJob = 0;
const stopped = new Set<number>();
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const mockLibraryTools: LibraryToolsApi = {
  async subscribe(handlers) {
    h = handlers;
    return () => (h = null);
  },
  async sortFacts(rows) {
    const f: T.SortFacts = { plays: {}, lastfm: lastfm ? {} : null, bpm: {} };
    for (const r of rows) {
      const n = hash(r.id);
      if (n % 3 === 0) f.bpm[r.id] = 78 + (n % 90);
      if (n % 4 !== 1) f.plays[r.id] = 1 + (n % 37);
      if (f.lastfm && n % 5 !== 0) f.lastfm[r.id] = 3 + (n % 412);
    }
    return f;
  },
  async years(ids) {
    const { mockCatalogue } = await kit();
    const job = ++yearsJob;
    const todo = [...new Set(ids)].filter((id) => !id.startsWith("ytm:"));
    void (async () => {
      let done = 0;
      for (const id of todo) {
        await wait(400);
        if (job !== yearsJob) return;
        done++;
        h?.year({ job, albumId: id, year: mockCatalogue.year(id), done, total: todo.length, finished: false, stopped: false });
      }
      h?.year({ job, albumId: null, year: null, done, total: todo.length, finished: true, stopped: false });
    })();
    return { job, known: {}, pending: todo.length, skipped: 0 };
  },
  async yearsStop() {
    yearsJob++;
  },
  async like(ids, liked) {
    const { mockApi, mockKit } = await kit();
    if (!mockKit.signedIn) throw { code: "authRequired", message: "Sign in to YouTube Music to use this.", retryable: false, action: "signIn" };
    if (!mockKit.online) throw { code: "network", message: "You're offline. Connect to the internet to change your likes.", retryable: true, action: null };
    const job = ++likeJob;
    void (async () => {
      let done = 0;
      for (const id of ids) {
        await wait(500);
        if (stopped.has(job)) break;
        const outcome = await mockApi.setLike(id, liked);
        done++;
        h?.like({ job, trackId: id, outcome, done, total: ids.length, finished: false, error: null });
      }
      h?.like({ job, trackId: null, outcome: null, done, total: ids.length, finished: true, error: null });
    })();
    return job;
  },
  async likeStop(job) {
    stopped.add(job);
  },
  async discography(links) {
    const { mockCatalogue, mockKit } = await kit();
    const artist = links[0]?.browseId.replace(/^MPAD/, "") ?? "";
    const wantAlbums = links.some((l) => l.params === "albums");
    const wantSingles = links.some((l) => l.params === "singles");
    const releases = [...(wantAlbums ? mockCatalogue.albumsOf(artist) : []), ...(wantSingles ? mockCatalogue.singlesOf(artist) : [])].sort((a, b) => (b.year ?? 0) - (a.year ?? 0));
    // Night Office's list "stops early", to show the qualified count.
    return mockKit.delay({ releases, complete: artist !== "night" }, 450);
  },
};
