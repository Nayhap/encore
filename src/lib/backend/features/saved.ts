// Saved (lane C): bookmarks (D25), saved queues (D33), folders and their covers (D34).
// This area's backend calls: the interface, and its Tauri side over the
// generated bindings (commands in src-tauri/src/features/saved.rs). The mock
// is ../mock/saved.ts. Reached as `call((a) => a.features.saved.…)`.
// Everything is local, per account, and works offline and signed out.
import { commands } from "../bindings";
import type { ArtRef, Bookmark, BookmarkTarget, CollectionCard, Folder, SavedQueueInfo } from "../types";
import { u } from "../unwrap";

export interface SavedApi {
  bookmarks(): Promise<Bookmark[]>;
  /** With `position`, a moment in the song. The same thing twice returns the one already kept. */
  bookmark(target: BookmarkTarget, position: number | null, title: string, subtitle: string, art: ArtRef | null): Promise<Bookmark>;
  unbookmark(id: string): Promise<void>;

  queues(): Promise<SavedQueueInfo[]>;
  /** The queue as it is now; the same name replaces that copy. */
  saveQueue(name: string): Promise<SavedQueueInfo>;
  /** Replaces the queue, paused at the saved song and position. */
  restoreQueue(id: string): Promise<void>;
  removeQueue(id: string): Promise<void>;

  folders(): Promise<Folder[]>;
  createFolder(name: string, items: CollectionCard[]): Promise<Folder>;
  renameFolder(id: string, name: string): Promise<void>;
  removeFolder(id: string): Promise<void>;
  setCollapsed(id: string, collapsed: boolean): Promise<void>;
  /** File a playlist or album here (out of any other folder). */
  file(id: string, card: CollectionCard): Promise<void>;
  unfile(id: string, item: string): Promise<void>;
  moveItem(id: string, item: string, before: string | null): Promise<void>;
  /** Opens the system's picker; false when cancelled. */
  chooseCover(id: string): Promise<boolean>;
  clearCover(id: string): Promise<void>;
  /** Opens the save sheet; false when cancelled. */
  exportFolder(id: string): Promise<boolean>;
  /** Opens the picker; null when cancelled. */
  importFolder(): Promise<Folder | null>;
}

export const tauriSaved: SavedApi = {
  bookmarks: () => u(commands.bookmarksList()) as any,
  bookmark: (target, position, title, subtitle, art) => u(commands.bookmarksAdd(target as any, position, title, subtitle, art)) as any,
  unbookmark: (id) => u(commands.bookmarksRemove(id)) as any,
  queues: () => u(commands.savedQueuesList()) as any,
  saveQueue: (name) => u(commands.savedQueueSave(name)) as any,
  restoreQueue: (id) => u(commands.savedQueueRestore(id)) as any,
  removeQueue: (id) => u(commands.savedQueueRemove(id)) as any,
  folders: () => u(commands.foldersList()) as any,
  createFolder: (name, items) => u(commands.foldersCreate(name, items as any)) as any,
  renameFolder: (id, name) => u(commands.foldersRename(id, name)) as any,
  removeFolder: (id) => u(commands.foldersRemove(id)) as any,
  setCollapsed: (id, collapsed) => u(commands.foldersSetCollapsed(id, collapsed)) as any,
  file: (id, card) => u(commands.foldersAdd(id, card as any)) as any,
  unfile: (id, item) => u(commands.foldersRemoveItem(id, item)) as any,
  moveItem: (id, item, before) => u(commands.foldersMoveItem(id, item, before)) as any,
  chooseCover: (id) => u(commands.foldersChooseCover(id)),
  clearCover: (id) => u(commands.foldersClearCover(id)) as any,
  exportFolder: (id) => u(commands.foldersExport(id)),
  importFolder: () => u(commands.foldersImport()) as any,
};
