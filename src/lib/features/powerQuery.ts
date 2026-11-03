// The Power Bar's reading of what was typed (D32): a scope prefix and a
// time to go to. No app state here, so `bun test` can run it on its own.

/** What the palette looks through. */
export type Scope = "all" | "commands" | "playlists" | "artists";

/** Typed first, a prefix narrows the palette (and leaves the box). */
export const PREFIXES: Record<string, Exclude<Scope, "all">> = { ">": "commands", "#": "playlists", "@": "artists" };

/** The scope a query's first character asks for, and the rest of it. */
export function splitScope(q: string): { scope: Scope; rest: string } | null {
  const s = PREFIXES[q[0] ?? ""];
  return s ? { scope: s, rest: q.slice(1).trimStart() } : null;
}

/** "2:15", "1:02:03", "go to 2:15", "seek 0:30" → seconds; anything else → null. */
export function parseTime(q: string): number | null {
  const m = /^(?:(?:go\s*to|goto|seek(?:\s*to)?|jump\s*to|at)\s+)?(?:(\d{1,2}):)?(\d{1,3}):(\d{2})$/i.exec(q.trim());
  if (!m) return null;
  const [h, min, sec] = [m[1] ? +m[1] : 0, +m[2], +m[3]];
  if (sec > 59 || (m[1] && min > 59)) return null;
  return h * 3600 + min * 60 + sec;
}
