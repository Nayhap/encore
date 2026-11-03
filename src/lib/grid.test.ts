// @ts-nocheck: run with `bun test src/lib` (bun's own runner and types).
import { expect, test } from "bun:test";
import { countLabel, kindsIn, view, yearOf } from "./grid";

const album = (id, title, subtitle, extra = {}) => ({ type: "collection", id, kind: "album", title, subtitle, artists: [{ name: "Mount Eerie" }], ...extra });
const items = [
  album("a", "Night Palace", "Album • 2024"),
  album("b", "Lost Wisdom, Pt. 2", "Album • 2019"),
  album("c", "Now Only", "Album", { year: 2018 }),
  album("d", "Sauna", "Album"),
  { type: "collection", id: "e", kind: "single", title: "Real Death", subtitle: "Single • 2016", artists: [] },
];

test("years come from the item, else from its subtitle", () => {
  expect(yearOf(items[0])).toBe(2024);
  expect(yearOf(items[2])).toBe(2018);
  expect(yearOf(items[3])).toBe(null);
});

test("newest and oldest put unknown years last", () => {
  expect(view(items, { kind: null, query: "", sort: "newest" }).map((i) => i.id)).toEqual(["a", "b", "c", "e", "d"]);
  expect(view(items, { kind: null, query: "", sort: "oldest" }).map((i) => i.id)).toEqual(["e", "c", "b", "a", "d"]);
});

test("find folds case and accents, kind filters, A–Z sorts", () => {
  expect(view(items, { kind: null, query: "palace NIGHT", sort: "default" }).map((i) => i.id)).toEqual(["a"]);
  expect(view(items, { kind: null, query: "eerie", sort: "default" }).length).toBe(4);
  expect(view(items, { kind: "single", query: "", sort: "default" }).map((i) => i.id)).toEqual(["e"]);
  expect(view(items, { kind: "album", query: "", sort: "title" }).map((i) => i.title)).toEqual(["Lost Wisdom, Pt. 2", "Night Palace", "Now Only", "Sauna"]);
});

test("kinds and counts", () => {
  expect(kindsIn(items)).toEqual([{ kind: "album", count: 4 }, { kind: "single", count: 1 }]);
  expect(countLabel(4, ["album"])).toBe("4 albums");
  expect(countLabel(1, ["album"])).toBe("1 album");
  expect(countLabel(5, ["album", "single"])).toBe("5 items");
});
