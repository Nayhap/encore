// Mock catalogue for `bun run dev` without Tauri. Every artist, song and
// lyric here is invented.

import type { Motif } from "./types";

export interface Art {
  seed: number;
  h1: number;
  h2: number;
  motif: Motif;
}

export interface LyricLine {
  t: number;
  text: string;
}

export interface MTrack {
  id: string;
  title: string;
  artistId: string;
  albumId: string;
  dur: number;
  explicit?: boolean;
  plays?: string;
  lyrics?: LyricLine[];
}

export type CollectionKind = "album" | "single" | "playlist" | "mix";

export interface MCollection {
  id: string;
  kind: CollectionKind;
  title: string;
  by: string;
  artistId?: string;
  year?: number;
  art: Art;
  trackIds: string[];
  desc?: string;
}

export interface MArtist {
  id: string;
  name: string;
  subs: string;
  art: Art;
  blurb: string;
}

const art = (seed: number, h1: number, h2: number, motif: Motif): Art => ({ seed, h1, h2, motif });

export const artists: MArtist[] = [
  { id: "lumen", name: "Lumen Drive", subs: "1.2M", art: art(11, 200, 260, "orbit"), blurb: "Synth-pop duo from Osaka writing songs for long drives home." },
  { id: "akane", name: "茜色ノート", subs: "845K", art: art(12, 14, 330, "sun"), blurb: "Akane-iro Note. Three-piece band, guitars first, choruses second." },
  { id: "pale", name: "Pale Harbor", subs: "392K", art: art(13, 186, 220, "wave"), blurb: "Slowcore from a seaside town that has seen better weather." },
  { id: "mirei", name: "Mirei", subs: "2.4M", art: art(14, 300, 340, "orbit"), blurb: "Vocalist and producer. Mostly midnight, occasionally noon." },
  { id: "night", name: "Night Office", subs: "128K", art: art(15, 48, 18, "grid"), blurb: "Instrumental hip-hop for the hours nobody bills." },
  { id: "velvet", name: "Velvet Arcade", subs: "610K", art: art(16, 330, 270, "blocks"), blurb: "City pop revivalists with a basement full of synthesizers." },
  { id: "sora", name: "Sora Kanata", subs: "1.9M", art: art(17, 160, 200, "stripes"), blurb: "Singer-songwriter. Acoustic guitar, big skies." },
  { id: "tide", name: "Tidewalker", subs: "77K", art: art(18, 220, 180, "wave"), blurb: "Ambient electronics recorded on a houseboat." },
  { id: "hymnal", name: "Neon Hymnal", subs: "455K", art: art(19, 280, 220, "grid"), blurb: "Choir samples, drum machines and a lot of reverb." },
  { id: "juniper", name: "Juniper Fields", subs: "233K", art: art(20, 96, 60, "sun"), blurb: "Folk songs about places that are no longer there." },
];

// Lyrics are deliberately short-lined so the song map has visible structure.
const daybreak: LyricLine[] = [
  [14, "Static on the dashboard, half past four"],
  [18.5, "Your voice is a rumour I can't ignore"],
  [23, "Streetlights keep counting what we left behind"],
  [27.5, "Every exit sign is a question mark"],
  [32, "Hold on, hold on"],
  [34.5, "The night's running out of road"],
  [39, "Turn the dial till the dark gives in"],
  [43.5, "There's a daybreak signal coming through"],
  [48, "Faint as a heartbeat, blue on blue"],
  [52.5, "If you can hear it, say my name"],
  [57, "We won't be strangers when the morning comes"],
  [61.5, "Daybreak signal, coming through"],
  [76, "Paper cups and a map we never read"],
  [80.5, "Radio ghosts singing what you said"],
  [85, "Every mile's a promise we forgot to keep"],
  [89.5, "Still I'm wide awake while the cities sleep"],
  [94, "Hold on, hold on"],
  [96.5, "The night's running out of road"],
  [101, "Turn the dial till the dark gives in"],
  [105.5, "There's a daybreak signal coming through"],
  [110, "Faint as a heartbeat, blue on blue"],
  [114.5, "If you can hear it, say my name"],
  [119, "We won't be strangers when the morning comes"],
  [123.5, "Daybreak signal, coming through"],
  [142, "And if the static swallows every word"],
  [147, "I'll draw the sunrise from the sounds I heard"],
  [152, "Ooh, coming through"],
  [156, "Ooh, coming through"],
  [160, "There's a daybreak signal coming through"],
  [164.5, "Faint as a heartbeat, blue on blue"],
  [169, "If you can hear it, say my name"],
  [173.5, "We won't be strangers when the morning comes"],
  [178, "Daybreak signal, coming through"],
  [186, "Blue on blue"],
  [192, "Coming through"],
].map(([t, text]) => ({ t: t as number, text: text as string }));

const yakou: LyricLine[] = [
  [12, "窓の外 ながれる街あかり"],
  [17, "眠れないまま 君をおもう"],
  [22, "時刻表には ない駅で"],
  [27, "ふたり 降りられたらいいのに"],
  [32, "夜行列車 星をこえて"],
  [37, "まだ知らない 朝へ はしる"],
  [42, "ことばより 確かなものを"],
  [47, "この胸に 抱いて ゆこう"],
  [64, "古いイヤホン 片方ずつ"],
  [69, "同じメロディ 違う夢"],
  [74, "トンネルぬけたら 言えるかな"],
  [79, "さよならじゃなく またねって"],
  [84, "夜行列車 星をこえて"],
  [89, "まだ知らない 朝へ はしる"],
  [94, "ことばより 確かなものを"],
  [99, "この胸に 抱いて ゆこう"],
  [118, "ゆれる灯り"],
  [123, "とおい汽笛"],
  [134, "夜行列車 星をこえて"],
  [139, "まだ知らない 朝へ はしる"],
  [144, "ことばより 確かなものを"],
  [149, "この胸に 抱いて ゆこう"],
].map(([t, text]) => ({ t: t as number, text: text as string }));

const glass: LyricLine[] = [
  [11, "Rain on the window writing out your name"],
  [16, "I keep the kettle on in case you came"],
  [21, "The forecast said there's nothing left to fear"],
  [26, "But I can see the sky from here"],
  [31, "It's glass weather"],
  [34, "Everything shows"],
  [37, "Every crack in the light"],
  [40, "Everybody knows"],
  [58, "Your coat's still hanging where you let it fall"],
  [63, "I hear the harbour breathing through the wall"],
  [68, "It's glass weather"],
  [71, "Everything shows"],
  [74, "Every crack in the light"],
  [77, "Everybody knows"],
  [104, "It's glass weather"],
  [107, "Everything shows"],
  [110, "Every crack in the light"],
  [113, "Everybody knows"],
].map(([t, text]) => ({ t: t as number, text: text as string }));

interface AlbumSeed {
  id: string;
  kind: CollectionKind;
  title: string;
  artistId: string;
  year: number;
  art: Art;
  songs: [string, number, Partial<MTrack>?][];
}

const albumSeeds: AlbumSeed[] = [
  {
    id: "al-daybreak", kind: "album", title: "Daybreak Signal", artistId: "lumen", year: 2026, art: art(101, 205, 250, "orbit"),
    songs: [
      ["Daybreak Signal", 218, { lyrics: daybreak, plays: "48M" }],
      ["Exit 41", 201, { plays: "12M" }],
      ["Paper Cups", 187],
      ["Radio Ghosts (feat. Mirei)", 244, { plays: "21M" }],
      ["Wide Awake While the Cities Sleep", 262],
    ],
  },
  {
    id: "al-yakou", kind: "album", title: "夜行列車", artistId: "akane", year: 2025, art: art(102, 18, 340, "sun"),
    songs: [
      ["夜行列車", 242, { lyrics: yakou, plays: "31M" }],
      ["片方のイヤホン", 215, { plays: "9.4M" }],
      ["トンネルの向こう側 - The Other Side of the Tunnel", 233],
      ["とおい汽笛", 198],
    ],
  },
  {
    id: "al-glass", kind: "album", title: "Glass Weather", artistId: "pale", year: 2024, art: art(103, 190, 215, "wave"),
    songs: [
      ["Glass Weather", 235, { lyrics: glass, plays: "6.1M" }],
      ["Kettle On", 204],
      ["Harbour Breathing", 281],
      ["Coat Still Hanging", 226],
    ],
  },
  {
    id: "al-mirei", kind: "album", title: "Noon Is a Rumour", artistId: "mirei", year: 2026, art: art(104, 305, 20, "orbit"),
    songs: [
      ["Noon Is a Rumour", 199, { plays: "64M", explicit: true }],
      ["Pink Static", 176, { plays: "38M" }],
      ["Overexposed", 210],
      ["Mirror, Mirror, Mirrorball", 187, { plays: "17M" }],
    ],
  },
  {
    id: "al-night", kind: "album", title: "Unbilled Hours", artistId: "night", year: 2025, art: art(105, 42, 12, "grid"),
    songs: [
      ["3:12 AM, Fourth Floor", 154],
      ["Fluorescent", 167],
      ["Printer Jam", 141],
      ["Overtime Lullaby", 189],
    ],
  },
  {
    id: "al-velvet", kind: "album", title: "Arcade Summer '89", artistId: "velvet", year: 2026, art: art(106, 335, 265, "blocks"),
    songs: [
      ["Neon Coast", 233, { plays: "22M" }],
      ["Highscore Heart", 214],
      ["Plastic Palm Trees", 241],
      ["Insert Coin to Continue", 205],
    ],
  },
  {
    id: "al-sora", kind: "album", title: "Big Sky Theory", artistId: "sora", year: 2025, art: art(107, 158, 200, "stripes"),
    songs: [
      ["Big Sky Theory", 223, { plays: "29M" }],
      ["空の色を覚えていて (Remember the Colour of the Sky)", 251, { plays: "15M" }],
      ["Kite String", 196],
    ],
  },
  {
    id: "al-tide", kind: "album", title: "Houseboat Tapes", artistId: "tide", year: 2024, art: art(108, 222, 176, "wave"),
    songs: [
      ["Mooring", 312],
      ["Low Tide, High Noon", 287],
      ["Bilge Pump Hymn", 266],
    ],
  },
  {
    id: "al-hymnal", kind: "album", title: "Cathedral Mode", artistId: "hymnal", year: 2026, art: art(109, 282, 228, "grid"),
    songs: [
      ["Cathedral Mode", 248, { plays: "8.8M" }],
      ["Choir.exe", 193],
      ["Vespers for a Drum Machine", 274],
    ],
  },
  {
    id: "al-juniper", kind: "album", title: "Places That Aren't There", artistId: "juniper", year: 2023, art: art(110, 92, 54, "sun"),
    songs: [
      ["The Mill Road", 208],
      ["Ferry That Stopped Running", 236],
      ["Juniper", 191, { plays: "4.2M" }],
    ],
  },
];

export const tracks: MTrack[] = [];
export const collections: MCollection[] = [];

for (const a of albumSeeds) {
  const ids: string[] = [];
  a.songs.forEach(([title, dur, extra], i) => {
    const id = `${a.id}-${i + 1}`;
    ids.push(id);
    tracks.push({ id, title, artistId: a.artistId, albumId: a.id, dur, ...extra });
  });
  collections.push({
    id: a.id, kind: a.kind, title: a.title, by: artists.find((x) => x.id === a.artistId)?.name ?? "Unknown artist", artistId: a.artistId,
    year: a.year, art: a.art, trackIds: ids,
  });
}

const t = (album: string, n: number) => `${album}-${n}`;

collections.push(
  {
    id: "pl-late", kind: "playlist", title: "Late Night Drive", by: "You", art: art(201, 215, 290, "orbit"),
    desc: "For the last hour before the city wakes up.",
    trackIds: [t("al-daybreak", 1), t("al-yakou", 1), t("al-glass", 1), t("al-velvet", 1), t("al-mirei", 2), t("al-night", 1), t("al-sora", 2), t("al-daybreak", 4), t("al-hymnal", 1), t("al-tide", 2), t("al-yakou", 3), t("al-velvet", 3)],
  },
  {
    id: "pl-liked", kind: "playlist", title: "Liked music", by: "Auto playlist", art: art(202, 340, 300, "blocks"),
    trackIds: [t("al-mirei", 1), t("al-daybreak", 1), t("al-yakou", 1), t("al-sora", 1), t("al-velvet", 2), t("al-glass", 1), t("al-juniper", 3), t("al-night", 4)],
  },
  {
    id: "mx-1", kind: "mix", title: "My Supermix", by: "Mirei, Lumen Drive, 茜色ノート and more", art: art(203, 300, 200, "stripes"),
    trackIds: [t("al-mirei", 2), t("al-daybreak", 2), t("al-yakou", 2), t("al-velvet", 4), t("al-sora", 3), t("al-hymnal", 2)],
  },
  {
    id: "mx-2", kind: "mix", title: "Focus Mix", by: "Night Office, Tidewalker, Neon Hymnal", art: art(204, 40, 200, "grid"),
    trackIds: [t("al-night", 1), t("al-tide", 1), t("al-night", 2), t("al-hymnal", 3), t("al-tide", 3), t("al-night", 3)],
  },
  {
    id: "mx-3", kind: "mix", title: "Discover Mix", by: "Juniper Fields, Pale Harbor, Sora Kanata", art: art(205, 120, 190, "sun"),
    trackIds: [t("al-juniper", 1), t("al-glass", 2), t("al-sora", 1), t("al-juniper", 2), t("al-glass", 3)],
  },
  {
    id: "pl-jp", kind: "playlist", title: "邦ロック 2026", by: "You", art: art(206, 8, 40, "blocks"),
    trackIds: [t("al-yakou", 1), t("al-yakou", 2), t("al-yakou", 3), t("al-sora", 2), t("al-yakou", 4)],
  },
  {
    id: "pl-rain", kind: "playlist", title: "songs that sound like rain on a bus window", by: "You", art: art(207, 200, 230, "wave"),
    trackIds: [t("al-glass", 1), t("al-glass", 3), t("al-tide", 1), t("al-juniper", 2)],
  },
);


export const moods = [
  { name: "Chill", h: 190 }, { name: "Commute", h: 30 }, { name: "Energize", h: 345 },
  { name: "Focus", h: 210 }, { name: "Feel good", h: 50 }, { name: "Party", h: 300 },
  { name: "Romance", h: 335 }, { name: "Sleep", h: 250 }, { name: "Workout", h: 10 },
  { name: "J-Pop", h: 320 }, { name: "City pop", h: 280 }, { name: "Indie & alternative", h: 140 },
];
