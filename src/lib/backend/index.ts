// The backend facade. Under Tauri it calls the generated commands; in a plain
// browser (`bun run dev`) it uses the mock backend so design work continues
// without a session.
import type * as T from "./types";

export * from "./types";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export class BackendError extends Error {
  code: string;
  action: string | null;
  retryable: boolean;
  constructor(e: T.AppError | { code: string; message: string; action?: string | null; retryable?: boolean }) {
    super(e.message);
    this.code = e.code;
    this.action = e.action ?? null;
    this.retryable = e.retryable ?? false;
  }
}

export interface Handlers {
  playerState(s: T.PlayerSnapshot): void;
  playerPosition(p: T.PositionAnchor): void;
  queue(q: T.QueueSplice): void;
  auth(a: T.AuthState): void;
  lyrics(l: T.LyricsEvent): void;
  artHue(h: T.ArtHueEvent): void;
  notice(n: T.NoticeEvent): void;
  signin(s: T.SigninEvent): void;
  /** A playlist was created, changed or deleted. */
  playlists(e: T.PlaylistsEvent): void;
  /** A play was reported to the history. */
  history(e: T.HistoryEvent): void;
}

export interface Api {
  /** Subscribe first, then bootstrap (D13). */
  subscribe(h: Handlers): Promise<() => void>;
  bootstrap(): Promise<T.Bootstrap>;
  resolverSetup(): Promise<T.ResolverStatus>;
  cacheInfo(): Promise<T.CacheInfo>;
  cacheClear(what: "audio" | "art" | "lyrics"): Promise<void>;
  home(): Promise<T.Page>;
  explore(): Promise<T.Page>;
  browsePage(browseId: string, params: string | null): Promise<T.Page>;
  moreShelves(cursor: string): Promise<T.Page>;
  search(query: string, filter: T.SearchFilter | null): Promise<T.SearchResults>;
  suggestions(query: string): Promise<string[]>;
  collection(id: string): Promise<T.Collection>;
  collectionMore(cursor: string): Promise<T.EntriesPage>;
  artist(id: string): Promise<T.Artist>;
  library(section: T.LibrarySection): Promise<T.LibraryPage>;
  libraryMore(section: T.LibrarySection, cursor: string): Promise<T.LibraryPage>;
  lyrics(track: T.Track): Promise<T.Lyrics | null>;
  playerSnapshot(): Promise<T.PlayerSnapshot>;
  queueSnapshot(): Promise<T.QueueSnapshot>;
  playTracks(tracks: T.Track[], start: number, source: T.QueueSource | null, shuffle: boolean): Promise<void>;
  playCollection(id: string, start: number, shuffle: boolean): Promise<void>;
  playTrack(track: T.Track): Promise<void>;
  startRadio(playlistId: string, label: string): Promise<void>;
  toggle(): Promise<void>;
  next(): Promise<void>;
  prev(): Promise<void>;
  seek(seconds: number): Promise<void>;
  jumpTo(uid: string): Promise<void>;
  playNext(tracks: T.Track[]): Promise<void>;
  addToQueue(tracks: T.Track[]): Promise<void>;
  queueRemove(uid: string): Promise<void>;
  queueMove(uid: string, before: string | null): Promise<void>;
  clearUpNext(): Promise<void>;
  setShuffle(on: boolean): Promise<void>;
  setRepeat(mode: T.Repeat): Promise<void>;
  setAutoplay(on: boolean): Promise<void>;
  setVolume(v: number): Promise<void>;
  setMuted(m: boolean): Promise<void>;
  setLike(trackId: string, liked: boolean): Promise<T.MutationOutcome>;
  setSaved(playlistId: string, saved: boolean): Promise<T.MutationOutcome>;
  setSubscribed(channelId: string, subscribed: boolean): Promise<T.MutationOutcome>;
  /** Refused as `duplicate` when a song is already there, unless `allowDuplicates`. */
  playlistAdd(playlistId: string, trackIds: string[], allowDuplicates: boolean): Promise<T.MutationOutcome>;
  playlistRemove(playlistId: string, entries: T.Entry[]): Promise<T.MutationOutcome>;
  playlistCreate(title: string, description: string, privacy: T.Privacy, trackIds: string[]): Promise<T.PlaylistCreated>;
  playlistEdit(playlistId: string, edit: T.PlaylistEdit): Promise<T.MutationOutcome>;
  playlistDelete(playlistId: string): Promise<T.MutationOutcome>;
  /** Move the row `setVideoId` before the row `before`, or to the end. */
  playlistMove(playlistId: string, setVideoId: string, before: string | null): Promise<T.MutationOutcome>;
  history(): Promise<T.HistoryPage>;
  historyMore(cursor: string): Promise<T.HistoryPage>;
  historyRemove(rowIds: string[]): Promise<T.MutationOutcome>;
  /** The other version of a track (song ↔ music video), or null. */
  counterpart(track: T.Track): Promise<T.Counterpart | null>;
  /** Switch a queue entry to its other version. */
  switchVersion(uid: string): Promise<void>;
  /** Play the other version of a track now, keeping the queue. */
  playVersion(track: T.Track): Promise<void>;
  setReportHistory(on: boolean): Promise<void>;
  authState(): Promise<T.AuthState>;
  browserProfiles(): Promise<T.BrowserProfile[]>;
  importBrowser(profileId: string): Promise<T.AuthState>;
  selectChannel(key: string): Promise<T.AuthState>;
  reloadChannels(): Promise<T.AccountChannel[]>;
  signOut(): Promise<void>;
  beginSignin(): Promise<void>;
  cancelSignin(): Promise<void>;
  /** URL for an art key at a given edge size. */
  artUrl(key: string, size: number): string;
}

let impl: Promise<Api> | null = null;

export function api(): Promise<Api> {
  if (!impl) impl = isTauri ? import("./tauri").then((m) => m.tauriApi) : import("./mock").then((m) => m.mockApi);
  return impl;
}

/** Run a backend call, turning failures into `BackendError`. */
export async function call<R>(f: (a: Api) => Promise<R>): Promise<R> {
  const a = await api();
  try {
    return await f(a);
  } catch (e) {
    if (e instanceof BackendError) throw e;
    if (e && typeof e === "object" && "code" in e && "message" in e) throw new BackendError(e as T.AppError);
    throw new BackendError({ code: "internal", message: e instanceof Error ? e.message : String(e) });
  }
}
