// Long text from YouTube, split for display.

export type Attribution = { source: string; sourceUrl: string; licence: string; licenceUrl: string | null };

/** YouTube ends many album and artist descriptions with a raw credit line:
 *  "From Wikipedia (https://…) under Creative Commons Attribution
 *  CC-BY-SA 3.0 (https://…)". Take it off the body so it can be shown as a
 *  short line with real links. Text without one comes back unchanged. */
export function splitAttribution(text: string): { body: string; credit: Attribution | null } {
  const m = /\s*From ([^()\n]+?) \((https?:\/\/[^\s)]+)\)(?: under ([^()\n]+?)(?: \((https?:\/\/[^\s)]+)\))?)?\.?\s*$/.exec(text);
  if (!m || m.index === 0) return { body: text, credit: null };
  const licence = (m[3] ?? "").trim();
  return {
    body: text.slice(0, m.index).trimEnd(),
    credit: { source: m[1].trim(), sourceUrl: m[2], licence: shortLicence(licence), licenceUrl: m[4] ?? null },
  };
}

/** "Creative Commons Attribution CC-BY-SA 3.0" → "CC BY-SA 3.0". */
function shortLicence(s: string): string {
  const cc = /CC[- ]?(BY(?:-[A-Z]{2})*)(?:\s*([\d.]+))?/i.exec(s);
  return cc ? `CC ${cc[1].toUpperCase()}${cc[2] ? ` ${cc[2]}` : ""}` : s;
}
