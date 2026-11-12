// Adding a long list of songs to a playlist (D34's "Save as playlist…"):
// pure, so it can be tested without a backend.

export type Sent = "done" | "rejected" | "unknown";

export interface AddResult {
  added: number;
  /** Songs YouTube refused on their own (gone, private, not for this country). */
  refused: string[];
  /** A request whose outcome couldn't be confirmed: nothing is sent after it
   *  (D20), and its songs may or may not be in the playlist. */
  unknown: boolean;
  stopped: boolean;
}

/** Add `ids` in order, `size` at a time. One refused song makes YouTube
 *  refuse its whole request, so a refused request is sent again as two
 *  halves, down to the song itself, which is then left out. After `giveUp`
 *  songs refused one by one, the rest is not tried: the playlist, not the
 *  songs, is being refused. */
export async function addAll(
  ids: string[],
  send: (ids: string[]) => Promise<Sent>,
  o: { size: number; giveUp?: number; stopped?: () => boolean; progress?: (added: number) => void },
): Promise<AddResult> {
  const r: AddResult = { added: 0, refused: [], unknown: false, stopped: false };
  const giveUp = o.giveUp ?? 25;
  const over = () => r.unknown || r.stopped || r.refused.length >= giveUp;
  const part = async (chunk: string[]): Promise<void> => {
    if (over()) return;
    if (o.stopped?.()) {
      r.stopped = true;
      return;
    }
    const sent = await send(chunk);
    if (sent === "done") {
      r.added += chunk.length;
      o.progress?.(r.added);
    } else if (sent === "unknown") r.unknown = true;
    else if (chunk.length === 1) r.refused.push(chunk[0]);
    else {
      const half = Math.ceil(chunk.length / 2);
      await part(chunk.slice(0, half));
      await part(chunk.slice(half));
    }
  };
  for (let i = 0; i < ids.length && !over(); i += o.size) await part(ids.slice(i, i + o.size));
  return r;
}
