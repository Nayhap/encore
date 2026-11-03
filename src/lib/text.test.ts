// @ts-nocheck: run with `bun test src/lib` (bun's own runner and types).
import { expect, test } from "bun:test";
import { splitAttribution } from "./text";

test("YouTube's Wikipedia credit becomes a short line", () => {
  const t =
    "The Glow Pt. 2 is the third studio album by the Microphones. From Wikipedia (https://en.wikipedia.org/wiki/The_Glow_Pt._2) under Creative Commons Attribution CC-BY-SA 3.0 (https://creativecommons.org/licenses/by-sa/3.0/legalcode)";
  const { body, credit } = splitAttribution(t);
  expect(body).toBe("The Glow Pt. 2 is the third studio album by the Microphones.");
  expect(credit).toEqual({
    source: "Wikipedia",
    sourceUrl: "https://en.wikipedia.org/wiki/The_Glow_Pt._2",
    licence: "CC BY-SA 3.0",
    licenceUrl: "https://creativecommons.org/licenses/by-sa/3.0/legalcode",
  });
});

test("text without a credit is left alone", () => {
  expect(splitAttribution("Songs for long drives.")).toEqual({ body: "Songs for long drives.", credit: null });
  expect(splitAttribution("Mixed from (https://example.com) and more")).toEqual({ body: "Mixed from (https://example.com) and more", credit: null });
});

test("a credit alone is not taken as the whole body", () => {
  const t = "From Wikipedia (https://en.wikipedia.org/wiki/X)";
  expect(splitAttribution(t)).toEqual({ body: t, credit: null });
});
