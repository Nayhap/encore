// @ts-nocheck: run with `bun test src/lib/features` (bun's own runner and types).
import { expect, test } from "bun:test";
import { parseTime, splitScope } from "./powerQuery";

test("times to go to", () => {
  expect(parseTime("2:15")).toBe(135);
  expect(parseTime("0:05")).toBe(5);
  expect(parseTime("go to 2:15")).toBe(135);
  expect(parseTime("Seek to 1:02:03")).toBe(3723);
  expect(parseTime("12:00")).toBe(720);
  expect(parseTime("2:75")).toBeNull();
  expect(parseTime("2:5")).toBeNull();
  expect(parseTime("1:75:00")).toBeNull();
  expect(parseTime("love 2:15")).toBeNull();
  expect(parseTime("abba")).toBeNull();
});

test("scope prefixes", () => {
  expect(splitScope(">sleep")).toEqual({ scope: "commands", rest: "sleep" });
  expect(splitScope("# road")).toEqual({ scope: "playlists", rest: "road" });
  expect(splitScope("@")).toEqual({ scope: "artists", rest: "" });
  expect(splitScope("daft punk")).toBeNull();
  expect(splitScope("")).toBeNull();
});
