<script lang="ts">
  // An artist's whole discography (D35): every release behind the artist's
  // "Albums" and "Singles & EPs" links (with their own params), year by
  // year. When YouTube stops listing early, the count says "at least".
  import { call, type CollectionCard, type PageLink } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { kindLabel } from "$lib/data";
  import { Resource } from "$lib/load.svelte";
  import Status from "$lib/components/Status.svelte";
  import Tile from "$lib/components/Tile.svelte";
  import Glyph from "./Glyph.svelte";

  let { links }: { links: PageLink[] } = $props();

  const res = new Resource(async () => {
    const d = await call((a) => a.features.libraryTools.discography(links));
    catalog.addItems(d.releases.map((c) => ({ type: "collection" as const, ...c })));
    return d;
  });

  type Kind = "all" | "albums" | "singles";
  let kind = $state<Kind>("all");
  let newest = $state(true);
  const isAlbum = (c: CollectionCard) => c.kind === "album";
  const all = $derived(res.data?.releases ?? []);
  const counts = $derived({ all: all.length, albums: all.filter(isAlbum).length, singles: all.filter((c) => !isAlbum(c)).length });
  const shown = $derived(kind === "all" ? all : all.filter((c) => (kind === "albums") === isAlbum(c)));
  const years = $derived.by(() => {
    const by = new Map<string, CollectionCard[]>();
    for (const c of shown) {
      const y = c.year ? String(c.year) : "Undated";
      by.set(y, [...(by.get(y) ?? []), c]);
    }
    const ys = [...by.keys()].sort((a, b) => (a === "Undated" ? 1 : b === "Undated" ? -1 : newest ? +b - +a : +a - +b));
    return ys.map((y) => ({ year: y, items: by.get(y)! }));
  });
  const n = (k: number) => k.toLocaleString();
  // One count for the whole discography, qualified when YouTube stopped early.
  const total = $derived(res.data ? `${res.data.complete ? "" : "At least "}${n(all.length)} ${all.length === 1 ? "release" : "releases"}` : "");
  const chips: [Kind, string][] = [
    ["all", "All"],
    ["albums", "Albums"],
    ["singles", "Singles and EPs"],
  ];
</script>

<Status loading={res.loading && !res.data} error={res.error} onretry={() => res.reload()} empty={res.data && !all.length ? "YouTube Music lists no releases for this artist." : null} />
{#if res.data && all.length}
  <div class="bar">
    <div class="chips" role="group" aria-label="Show">
      {#each chips as [k, label]}
        {#if k === "all" || counts[k]}
          <button class="chip" aria-pressed={kind === k} onclick={() => (kind = k)}>{label}</button>
        {/if}
      {/each}
    </div>
    <div class="end">
      <span class="total">{total}</span>
      <button class="tool" onclick={() => (newest = !newest)} aria-label={newest ? "Newest first. Show oldest first" : "Oldest first. Show newest first"} title={newest ? "Show oldest first" : "Show newest first"}>
        <Glyph name={newest ? "sort" : "sortUp"} />
        <span>{newest ? "Newest first" : "Oldest first"}</span>
      </button>
    </div>
  </div>
  {#each years as g (g.year)}
    <section class="year">
      <h3>{g.year}</h3>
      <div class="grid">
        {#each g.items as c (c.id)}
          <Tile item={{ type: "collection", ...c, subtitle: kindLabel[c.kind] }} size={168} />
        {/each}
      </div>
    </section>
  {/each}
  {#if !res.data.complete}
    <p class="note">YouTube Music stopped listing here, so there may be more.</p>
  {/if}
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 4px 0 18px;
  }
  .chips {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .end {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .total {
    font-size: 13px;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .tool {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    padding: 0 12px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 600;
    color: var(--mute);
    white-space: nowrap;
    transition: color 0.15s, background-color 0.15s;
  }
  .tool:hover {
    color: var(--text);
    background: var(--hover);
  }
  .year {
    display: grid;
    grid-template-columns: 76px minmax(0, 1fr);
    gap: 20px;
    padding: 18px 0;
    border-top: 1px solid var(--line);
  }
  /* Under the artist page's sticky tabs. */
  h3 {
    position: sticky;
    top: 58px;
    align-self: start;
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 650;
    letter-spacing: -0.03em;
    font-variation-settings: "wdth" 85;
    color: var(--mute);
    font-variant-numeric: tabular-nums;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, 168px);
    gap: 22px 20px;
  }
  .note {
    margin-top: 10px;
    font-size: 13px;
    color: var(--mute);
  }
  @media (max-width: 760px) {
    .year {
      grid-template-columns: minmax(0, 1fr);
      gap: 10px;
    }
  }
</style>
