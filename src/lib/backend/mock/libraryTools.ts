// The mock of library tools (lane B) for `bun run dev`. Shared mock state
// (the simulated player, the catalogue, the event handlers) is `mockKit`
// in ../mock.ts.
import type { LibraryToolsApi } from "../features/libraryTools";

export const mockLibraryTools: LibraryToolsApi = {};
