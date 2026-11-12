import { expect, test } from "bun:test";
import { addAll, type Sent } from "./copyList";

const ids = (n: number) => Array.from({ length: n }, (_, i) => `s${i}`);

/** A playlist that refuses any request holding one of `bad`. */
function playlist(bad: string[] = []) {
  const got: string[] = [];
  let requests = 0;
  const send = async (c: string[]): Promise<Sent> => {
    requests++;
    if (c.some((x) => bad.includes(x))) return "rejected";
    got.push(...c);
    return "done";
  };
  return { got, send, requests: () => requests };
}

test("a long list goes in whole and in order", async () => {
  const p = playlist();
  const r = await addAll(ids(1352), p.send, { size: 200 });
  expect(r).toEqual({ added: 1352, refused: [], unknown: false, stopped: false });
  expect(p.got).toEqual(ids(1352));
  expect(p.requests()).toBe(7);
});

test("a song YouTube refuses is left out, the rest keep their order", async () => {
  const p = playlist(["s3", "s250"]);
  const r = await addAll(ids(400), p.send, { size: 200 });
  expect(r.refused).toEqual(["s3", "s250"]);
  expect(r.added).toBe(398);
  expect(p.got).toEqual(ids(400).filter((x) => x !== "s3" && x !== "s250"));
});

test("a playlist that refuses everything is given up on", async () => {
  let n = 0;
  const r = await addAll(ids(1000), async () => (n++, "rejected"), { size: 200, giveUp: 5 });
  expect(r.added).toBe(0);
  expect(r.refused.length).toBe(5);
  expect(n < 40).toBe(true);
});

test("nothing is sent after an unconfirmed request, or after a stop", async () => {
  let n = 0;
  const r = await addAll(ids(600), async () => (++n === 2 ? "unknown" : "done"), { size: 200 });
  expect([r.added, r.unknown, n]).toEqual([200, true, 2]);
  let m = 0;
  const s = await addAll(ids(600), async () => (m++, "done"), { size: 200, stopped: () => m >= 1 });
  expect([s.added, s.stopped, m]).toEqual([200, true, 1]);
});
