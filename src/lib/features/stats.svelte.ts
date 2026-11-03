// Stats (lane D): the Stats tab over the local play log (D30).
//
// This module is the area's UI state and actions (`call((a) => a.features.stats.…)`). It registers
// its menu items, palette commands, chips and keys with ./hooks at load
// time, and is re-exported from $lib/state.svelte. It is loaded while
// state.svelte.ts is still loading: read `player`, `ui` and the rest only
// inside functions and getters, never at the top level or in a field
// initialiser.
import { call, type StatsPeriod } from "$lib/backend";
import { registerFeature } from "./hooks";

class Listening {
  /** Bumps when a play is counted or ends, or the log is cleared. */
  rev = $state(0);
}

/** The local play log, as the History page's Stats tab and Settings show it. */
export const listening = new Listening();

let timer: ReturnType<typeof setTimeout> | undefined;

registerFeature({
  name: "stats",
  // Plays end in bursts (skipping through a queue): refresh once they settle.
  subscribe: () =>
    call((a) =>
      a.features.stats.subscribe(() => {
        clearTimeout(timer);
        timer = setTimeout(() => listening.rev++, 1500);
      }),
    ),
});

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** "November", "November 2025" (another year), or "2026". */
export function periodName(p: StatsPeriod, short = false) {
  if (p.month == null) return String(p.year);
  const m = short ? MONTHS[p.month - 1].slice(0, 3) : MONTHS[p.month - 1];
  return p.year === new Date().getFullYear() ? m : `${m} ${p.year}`;
}

/** "12 h 40 min", "35 min". */
export function minutesText(min: number | null | undefined) {
  const m = Math.round(min ?? 0);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h.toLocaleString()} h ${m % 60} min` : `${h.toLocaleString()} h`;
}
