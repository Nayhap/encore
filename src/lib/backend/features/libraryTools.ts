// Library tools (lane B): find, sort, select and mass actions, duplicates (D34), the discography and community playlists (D35).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/library_tools.rs). The mock
// is ../mock/libraryTools.ts. Reached as `call((a) => a.features.libraryTools.…)`.
// Methods return promises of binding types (named in ../types.ts); events
// come through a `subscribe(h): Promise<() => void>` of its own.
import { commands, events } from "../bindings";
import type * as T from "../types";
import { u } from "../unwrap";

export interface LibraryToolsHandlers {
  /** An album's year arrived, or a lookup ended. */
  year: (e: T.LibraryYearEvent) => void;
  /** A song of a like job was answered, or the job ended. */
  like: (e: T.LibraryLikeEvent) => void;
}

export interface LibraryToolsApi {
  subscribe(h: LibraryToolsHandlers): Promise<() => void>;
  /** Your plays, Last.fm counts and tempos of these rows, from what Encore keeps. */
  sortFacts(rows: T.SortRow[]): Promise<T.SortFacts>;
  /** Release years of albums: cached ones now, the rest as `year` events. */
  years(albumIds: string[]): Promise<T.YearsStart>;
  yearsStop(): Promise<void>;
  /** Like or unlike many songs, paced; returns the job. */
  like(trackIds: string[], liked: boolean): Promise<number>;
  likeStop(job: number): Promise<void>;
  /** Every release behind an artist's release links (D35). */
  discography(links: T.PageLink[]): Promise<T.Discography>;
}

export const tauriLibraryTools: LibraryToolsApi = {
  async subscribe(h) {
    const offs = await Promise.all([events.libraryYearEvent.listen((e) => h.year(e.payload as any)), events.libraryLikeEvent.listen((e) => h.like(e.payload as any))]);
    return () => offs.forEach((off) => off());
  },
  sortFacts: (rows) => u(commands.librarySortFacts(rows as any)) as any,
  years: (ids) => u(commands.libraryYears(ids)) as any,
  yearsStop: () => u(commands.libraryYearsStop()) as any,
  like: (ids, liked) => u(commands.libraryLike(ids, liked)),
  likeStop: (job) => u(commands.libraryLikeStop(job)) as any,
  discography: (links) => u(commands.libraryDiscography(links as any)) as any,
};
