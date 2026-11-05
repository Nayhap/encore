// @ts-nocheck: run with `bun test src/lib` (bun's own runner and types).
import { expect, test } from "bun:test";
import { bezier, ease, fadeTimes, hueAt } from "./hue";

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

test("no step of the fade is a jump", () => {
  const times = fadeTimes();
  expect(times[0]).toBeLessThanOrEqual(20);
  expect(times.length).toBeLessThan(30);
  // A half-turn, the largest change a fade makes.
  let last = 0;
  let largest = 0;
  for (const ms of times) {
    const [h] = hueAt([0, 0], [180, 0], ms / 1200);
    largest = Math.max(largest, h - last);
    last = h;
  }
  expect(last).toBe(180);
  expect(largest).toBeLessThanOrEqual(22);
});
