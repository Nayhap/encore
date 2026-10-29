// Tauri implementation of the backend facade over the generated bindings.
import { protocolUrl } from "$lib/platform";
import { commands, events } from "./bindings";
import { BackendError, type Api, type Handlers } from "./index";

type R<T> = { status: "ok"; data: T } | { status: "error"; error: any };

async function u<T>(p: Promise<R<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new BackendError(r.error);
  return r.data;
}

export const tauriApi: Api = {
  async subscribe(h: Handlers) {
    const offs = await Promise.all([
      events.playerStateEvent.listen((e) => h.playerState(e.payload as any)),
      events.playerPositionEvent.listen((e) => h.playerPosition(e.payload as any)),
      events.queueEvent.listen((e) => h.queue(e.payload as any)),
      events.authEvent.listen((e) => h.auth(e.payload as any)),
      events.lyricsEvent.listen((e) => h.lyrics(e.payload as any)),
      events.artHueEvent.listen((e) => h.artHue(e.payload as any)),
      events.noticeEvent.listen((e) => h.notice(e.payload as any)),
      events.signinEvent.listen((e) => h.signin(e.payload as any)),
    ]);
    return () => offs.forEach((off) => off());
  },
  bootstrap: () => u(commands.bootstrap()) as any,
  resolverSetup: () => u(commands.resolverSetup()) as any,
  cacheInfo: () => u(commands.cacheInfo()),
  cacheClear: (w) => u(commands.cacheClear(w)) as any,
  home: () => u(commands.home()) as any,
  explore: () => u(commands.explore()) as any,
  browsePage: (b, p) => u(commands.browsePage(b, p)) as any,
  moreShelves: (c) => u(commands.moreShelves(c)) as any,
  search: (q, f) => u(commands.search(q, f)) as any,
  suggestions: (q) => u(commands.suggestions(q)),
  collection: (id) => u(commands.collection(id)) as any,
  collectionMore: (c) => u(commands.collectionMore(c)) as any,
  artist: (id) => u(commands.artist(id)) as any,
  library: (s) => u(commands.library(s)) as any,
  libraryMore: (s, c) => u(commands.libraryMore(s, c)) as any,
  lyrics: (t) => u(commands.lyrics(t)) as any,
  playerSnapshot: () => u(commands.playerSnapshot()) as any,
  queueSnapshot: () => u(commands.queueSnapshot()) as any,
  playTracks: (t, s, src, sh) => u(commands.playTracks(t, s, src, sh)) as any,
  playCollection: (id, s, sh) => u(commands.playCollection(id, s, sh)) as any,
  playTrack: (t) => u(commands.playTrack(t)) as any,
  startRadio: (p, l) => u(commands.startRadio(p, l)) as any,
  toggle: () => u(commands.toggle()) as any,
  next: () => u(commands.next()) as any,
  prev: () => u(commands.prev()) as any,
  seek: (s) => u(commands.seek(s)) as any,
  jumpTo: (uid) => u(commands.jumpTo(uid)) as any,
  playNext: (t) => u(commands.playNext(t)) as any,
  addToQueue: (t) => u(commands.addToQueue(t)) as any,
  queueRemove: (uid) => u(commands.queueRemove(uid)) as any,
  queueMove: (uid, b) => u(commands.queueMove(uid, b)) as any,
  clearUpNext: () => u(commands.clearUpNext()) as any,
  setShuffle: (on) => u(commands.setShuffle(on)) as any,
  setRepeat: (m) => u(commands.setRepeat(m)) as any,
  setAutoplay: (on) => u(commands.setAutoplay(on)) as any,
  setVolume: (v) => u(commands.setVolume(v)) as any,
  setMuted: (m) => u(commands.setMuted(m)) as any,
  setLike: (id, l) => u(commands.setLike(id, l)),
  setSaved: (p, s) => u(commands.setSaved(p, s)),
  setSubscribed: (c, s) => u(commands.setSubscribed(c, s)),
  playlistAdd: (p, ids) => u(commands.playlistAdd(p, ids)),
  playlistRemove: (p, e) => u(commands.playlistRemove(p, e)),
  authState: () => u(commands.authState()) as any,
  browserProfiles: () => u(commands.browserProfiles()),
  importBrowser: (p) => u(commands.importBrowser(p)) as any,
  selectChannel: (k) => u(commands.selectChannel(k)) as any,
  reloadChannels: () => u(commands.reloadChannels()) as any,
  signOut: () => u(commands.signOut()) as any,
  beginSignin: () => u(commands.beginSignin()) as any,
  cancelSignin: () => u(commands.cancelSignin()) as any,
  artUrl: (key, size) => protocolUrl("art", `${key}?s=${size}`),
};
