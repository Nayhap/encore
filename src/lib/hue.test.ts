// @ts-nocheck: run with `bun test src/lib` (bun's own runner and types).
import { expect, test } from "bun:test";
import { bezier, ease, hueAt } from "./hue";

test("the curve starts at 0, ends at 1 and climbs", () => {
  expect(ease(0)).toBe(0);
  expect(ease(1)).toBe(1);
  let last = 0;
  for (let i = 1; i < 20; i++) {
    const v = ease(i / 20);
    expect(v).toBeGreaterThan(last);
    last = v;
  }
  // --ease leaves early and settles late: most of the way there by half time.
  expect(ease(0.5)).toBeGreaterThan(0.85);
});

test("a linear curve is the identity", () => {
  const lin = bezier(0, 0, 1, 1);
  for (const x of [0.1, 0.33, 0.5, 0.9]) expect(lin(x)).toBeCloseTo(x, 4);
});

test("hues go straight across the numbers, as the CSS transition did", () => {
  expect(hueAt([350, 30], [10, 50], 0)).toEqual([350, 30]);
  expect(hueAt([350, 30], [10, 50], 1)).toEqual([10, 50]);
  const [h] = hueAt([350, 30], [10, 50], 0.5);
  expect(h).toBeLessThan(350);
  expect(h).toBeGreaterThan(10);
});
