// UI-facing names for the generated binding types (values received from Rust).
import type * as B from "./bindings";

export type ItemId = B.ItemId;
export type Track = B.Track_Serialize;
export type Credit = B.Credit_Serialize;
export type ArtRef = B.ArtRef_Serialize;
export type Entry = B.Entry_Serialize;
export type Collection = B.Collection_Serialize;
export type CollectionCard = B.CollectionCard_Serialize;
export type CollectionKind = B.CollectionKind;
export type ArtistCard = B.ArtistCard_Serialize;
export type Artist = B.Artist_Serialize;
export type Shelf = B.Shelf_Serialize;
export type ShelfItem = B.ShelfItem_Serialize;
export type PageLink = B.PageLink_Serialize;
export type Page = B.Page_Serialize;
export type Chip = B.Chip_Serialize;
export type SearchResults = B.SearchResults_Serialize;
export type SearchFilter = B.SearchFilter;
export type LibrarySection = B.LibrarySection;
export type LibraryPage = B.LibraryPage_Serialize;
export type Lyrics = B.Lyrics_Serialize;
export type LyricLine = B.LyricLine_Serialize;
export type AuthState = B.AuthState_Serialize;
export type AccountChannel = B.AccountChannel_Serialize;
export type BrowserProfile = B.BrowserProfile;
export type QueueEntry = B.QueueEntry_Serialize;
export type QueueSource = B.QueueSource_Serialize;
export type Repeat = B.Repeat;
export type PlayerSnapshot = B.PlayerSnapshot_Serialize;
export type QueueSplice = B.QueueSplice_Serialize;
export type QueueSnapshot = B.QueueSnapshot_Serialize;
export type PositionAnchor = B.PositionAnchor_Serialize;
export type Phase = B.Phase;
export type AppError = B.AppError_Serialize;
export type MutationOutcome = B.MutationOutcome;
export type ResolverStatus = B.ResolverStatus_Serialize;
export type Bootstrap = B.Bootstrap_Serialize;
export type CacheInfo = B.CacheInfo;
export type EntriesPage = B.EntriesPage_Serialize;
export type SigninEvent = B.SigninEvent_Serialize;
export type NoticeEvent = B.NoticeEvent_Serialize;
export type LyricsEvent = B.LyricsEvent_Serialize;
export type ArtHueEvent = B.ArtHueEvent;
export type Privacy = B.Privacy;
export type PlaylistEdit = B.PlaylistEdit_Deserialize;
export type PlaylistCreated = B.PlaylistCreated_Serialize;
export type HistoryPage = B.HistoryPage_Serialize;
export type HistoryGroup = B.HistoryGroup_Serialize;
export type HistoryRow = B.HistoryRow_Serialize;
export type Counterpart = B.Counterpart_Serialize;
export type SharedSegment = B.SharedSegment;
export type PlaylistsEvent = B.PlaylistsEvent_Serialize;
export type HistoryEvent = B.HistoryEvent;

/** Mock-only procedural artwork (dev without Tauri). */
export type Motif = "orbit" | "stripes" | "grid" | "wave" | "sun" | "blocks";
export interface MockArt {
  seed: number;
  h1: number;
  h2: number;
  motif: Motif;
}
/** What `Cover` can draw: a real art key, mock art, or nothing. */
export type AnyArt = ArtRef | MockArt | null | undefined;
