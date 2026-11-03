// `bun test src/lib`: the library tools' finding, sorting and duplicates.
import { describe, expect, test } from "bun:test";
import type { Entry, Track } from "$lib/backend";
import { bareTitle, duplicates, find, fold, known, noFacts, playCount, sortEntries } from "./logic";

const track = (id: string, title: string, artist: string, extra: Partial<Track> = {}): Track => ({
  id,
  title,
  artists: [{ name: artist, id: null }],
  album: null,
  duration: 200,
  explicit: false,
  art: null,
  kind: "song",
  plays: null,
  liked: null,
  playable: true,
  counterpart: null,
  ...extra,
});
let n = 0;
const entry = (t: Track, videoId = t.id): Entry => ({ entryId: `e${++n}`, track: t, setVideoId: `sv${n}`, videoId });
const titles = (es: Entry[]) => es.map((e) => e.track.title);

describe("find", () => {
  const es = [entry(track("a", "Voyager", "Daft Punk", { album: { name: "Discovery", id: null } })), entry(track("b", "Halo", "Beyoncé")), entry(track("c", "Digital Love", "Daft Punk"))];
  test("every word, anywhere in title, artist or album", () => {
    expect(titles(find(es, "daft love"))).toEqual(["Digital Love"]);
    expect(titles(find(es, "discovery"))).toEqual(["Voyager"]);
  });
  test("accents and case don't matter", () => {
    expect(titles(find(es, "BEYONCE"))).toEqual(["Halo"]);
    expect(fold("  Ünïcode   Spaces ")).toBe("unicode spaces");
  });
  test("an empty query is everything", () => {
    expect(find(es, "   ")).toBe(es);
  });
});

describe("sort", () => {
  const es = [
    entry(track("a", "beta", "Zed", { duration: 300, plays: "1.2M plays", album: { name: "X", id: "alb1" } })),
    entry(track("b", "Alpha", "amy", { duration: null, plays: "950K plays", album: { name: "Y", id: "alb2" } })),
    entry(track("c", "gamma", "Bob", { duration: 100, plays: null })),
    entry(track("d", "Alpha", "Cy", { duration: 100, plays: "2B views" })),
  ];
  test("words A to Z ignoring case, ties keep playlist order", () => {
    expect(sortEntries(es, "title", false, noFacts()).map((e) => e.track.id)).toEqual(["b", "d", "a", "c"]);
    expect(sortEntries(es, "artist", false, noFacts()).map((e) => e.track.id)).toEqual(["b", "c", "d", "a"]);
  });
  test("unknown values go last both ways", () => {
    expect(sortEntries(es, "duration", false, noFacts()).map((e) => e.track.id)).toEqual(["c", "d", "a", "b"]);
    expect(sortEntries(es, "duration", true, noFacts()).map((e) => e.track.id)).toEqual(["a", "c", "d", "b"]);
  });
  test("YouTube plays read the service's text", () => {
    expect(playCount("48M plays")).toBe(48e6);
    expect(playCount("12,345 plays")).toBe(12345);
    expect(playCount("1.2B views")).toBe(1.2e9);
    expect(playCount(null)).toBeNull();
    expect(sortEntries(es, "youtube", true, noFacts()).map((e) => e.track.id)).toEqual(["d", "a", "b", "c"]);
  });
  test("years come from the albums, plays and tempo from the facts", () => {
    const f = noFacts();
    f.years.set("alb1", 2001).set("alb2", null);
    f.plays = { c: 3, a: 9 };
    f.bpm = { d: 128, b: 90 };
    expect(sortEntries(es, "year", true, f).map((e) => e.track.id)).toEqual(["a", "b", "c", "d"]);
    expect(sortEntries(es, "plays", true, f).map((e) => e.track.id)).toEqual(["a", "c", "b", "d"]);
    expect(sortEntries(es, "bpm", false, f).map((e) => e.track.id)).toEqual(["b", "d", "a", "c"]);
    expect(known(es, "bpm", f)).toBe(2);
    expect(known(es, "lastfm", f)).toBe(0);
  });
  test("playlist order, reversed or not", () => {
    expect(sortEntries(es, "order", false, noFacts())).toBe(es);
    expect(sortEntries(es, "order", true, noFacts()).map((e) => e.track.id)).toEqual(["d", "c", "b", "a"]);
  });
});

describe("duplicates", () => {
  test("later copies of the same row are copies", () => {
    const a = track("a", "One", "X");
    const es = [entry(a), entry(track("b", "Two", "X")), entry(a), entry(a)];
    const d = duplicates(es);
    expect([...d.copies]).toEqual([es[2].entryId, es[3].entryId]);
    expect(d.rows.map((e) => e.entryId)).toEqual([es[0].entryId, es[2].entryId, es[3].entryId]);
    expect(d.versions.size).toBe(0);
  });
  test("a song and its video are versions, linked or by name", () => {
    const song = track("s", "Voyager", "Daft Punk");
    const video = track("v", "Voyager (Official Video)", "Daft Punk", { kind: "video", duration: 230 });
    const linked = track("s2", "Digital Love", "Daft Punk", { counterpart: "v2" });
    const es = [entry(song), entry(track("x", "Other", "Y")), entry(video), entry(linked, "v2-row"), entry(track("v2", "Digital Love", "Daft Punk", { kind: "video" }))];
    const d = duplicates(es);
    expect(d.copies.size).toBe(0);
    expect([...d.versions]).toEqual([es[2].entryId, es[4].entryId]);
    expect(d.kind.get(es[2].entryId)).toBe("version");
  });
  test("two songs of the same name are not versions", () => {
    const d = duplicates([entry(track("a", "Intro", "X")), entry(track("b", "Intro", "X"))]);
    expect(d.rows.length).toBe(0);
  });
  test("a copy of a version joins its group once", () => {
    const song = track("s", "Voyager", "Daft Punk");
    const video = track("v", "Voyager", "Daft Punk", { kind: "video" });
    const es = [entry(song), entry(video), entry(video)];
    const d = duplicates(es);
    expect(d.rows.map((e) => e.entryId)).toEqual(es.map((e) => e.entryId));
    expect(d.copies.has(es[2].entryId)).toBe(true);
  });
  test("version labels are not part of the title", () => {
    expect(bareTitle("Get Lucky (Official Audio)")).toBe("get lucky");
    expect(bareTitle("Get Lucky [Remastered 2013]")).toBe("get lucky");
    expect(bareTitle("Get Lucky - ")).toBe("get lucky");
  });
});
