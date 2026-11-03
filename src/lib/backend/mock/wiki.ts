// The mock of wiki (lane E) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts. Tidewalker has no YouTube description in the mock, so its
// page shows the Wikipedia summary (D29); the other artists keep theirs.
import type { WikiApi } from "../features/wiki";
import { mockKit } from "../mock";
import * as M from "../mockdata";
import type * as T from "../types";

const tide = M.artists.find((a) => a.id === "tide");
if (tide) tide.blurb = "";

const pages: Record<string, T.WikiExtract> = {
  Tidewalker: {
    title: "Tidewalker (band)",
    description: "Scottish ambient electronic duo",
    extract:
      "Tidewalker is an ambient electronic duo formed in Leith in 2014 by Isla Reid and Tomás Varga. The pair record on a converted houseboat moored in the Firth of Forth, building tracks from field recordings of the water, modular synthesisers and a detuned harmonium. Their third album, Slack Water, was shortlisted for the Scottish Album of the Year Award.",
    url: "https://en.wikipedia.org/wiki/Tidewalker_(band)",
    lang: "en",
  },
};

export const mockWiki: WikiApi = {
  artist: (_id, name) => mockKit.delay(pages[name] ?? null, mockKit.online ? 450 : 60),
};
