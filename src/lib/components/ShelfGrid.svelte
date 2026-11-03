<script lang="ts">
  // A "Show all" page: one shelf laid out as a grid, with Find, the kinds it
  // holds as chips (only when there's more than one), and a sort.
  import type { ShelfItem } from "$lib/backend";
  import { ui, type MenuItem } from "$lib/state.svelte";
  import { countLabel, kindNames, kindsIn, view, type GridSort } from "$lib/grid";
  import Glyph from "./library-tools/Glyph.svelte";
  import Icon from "./Icon.svelte";
  import Tile from "./Tile.svelte";

  let { items }: { items: ShelfItem[] } = $props();

  let query = $state("");
  let kind = $state<string | null>(null);
  let sort = $state<GridSort>("default");

  const kinds = $derived(kindsIn(items));
  const dated = $derived(items.some((it) => (it.type === "collection" && (it.year || /\b(19|20)\d{2}\b/.test(it.subtitle)))));
  const shown = $derived(view(items, { kind, query, sort }));
  // Find and Sort earn their place on a long page only.
  const tools = $derived(items.length >= 8);
  const count = $derived(countLabel(shown.length, kind ? [kind] : kinds.map((k) => k.kind)));

  const sortLabels: Record<GridSort, string> = { default: "YouTube's order", newest: "Newest first", oldest: "Oldest first", title: "Title, A to Z" };
  function sortMenu(e: MouseEvent) {
    const keys: GridSort[] = dated ? ["default", "newest", "oldest", "title"] : ["default", "title"];
    const menu: MenuItem[] = keys.map((k) => ({ label: sortLabels[k], icon: sort === k ? "check" : "none", run: () => (sort = k) }));
    ui.openMenu(e, menu);
  }
</script>

{#if tools || kinds.length > 1}
<div class="bar">
  {#if tools}
  <label class="find" class:filled={!!query}>
    <Icon name="search" size={15} />
    <input bind:value={query} placeholder="Find" aria-label="Find on this page" spellcheck="false" autocomplete="off" onkeydown={(e) => e.key === "Escape" && (query = "")} />
    {#if query}<button class="clear" aria-label="Clear" onclick={() => (query = "")}><Icon name="x" size={13} /></button>{/if}
  </label>
  {/if}
  {#if kinds.length > 1}
    <div class="chips" role="group" aria-label="Show">
      <button class="chip" aria-pressed={kind === null} onclick={() => (kind = null)}>All</button>
      {#each kinds as k (k.kind)}
        <button class="chip" aria-pressed={kind === k.kind} onclick={() => (kind = kind === k.kind ? null : k.kind)}>{kindNames[k.kind]?.[1] ?? k.kind}</button>
      {/each}
    </div>
  {/if}
  <div class="end">
    <span class="count">{count}</span>
    {#if tools}<button class="tool" class:on={sort !== "default"} onclick={sortMenu} aria-label="Sort: {sortLabels[sort]}">
      <Glyph name={sort === "oldest" ? "sortUp" : "sort"} />
      <span>{sort === "default" ? "Sort" : sortLabels[sort]}</span>
    </button>{/if}
  </div>
</div>
{/if}

{#if shown.length}
  <div class="grid">
    {#each shown as it, i (it.type === "link" ? it.browseId + i : it.id)}
      <Tile item={it} fluid />
    {/each}
  </div>
{:else}
  <p class="none">{query ? `Nothing here matches “${query}”.` : "Nothing of that kind here."} <button onclick={() => ((query = ""), (kind = null))}>Show everything</button></p>
{/if}

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 12px;
    margin: 6px 0 22px;
  }
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 240px;
    min-width: 140px;
    height: 34px;
    padding: 0 6px 0 12px;
    border-radius: 999px;
    color: var(--mute);
    background: color-mix(in srgb, var(--text) 5%, transparent);
    box-shadow: inset 0 0 0 1px var(--line);
    transition: background-color 0.15s, box-shadow 0.15s;
    cursor: text;
  }
  .find:hover {
    background: color-mix(in srgb, var(--text) 7%, transparent);
  }
  .find:focus-within,
  .find.filled {
    color: var(--text);
  }
  .find:focus-within {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 70%, transparent);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    background: none;
    font-size: 13.5px;
    -webkit-user-select: text;
    user-select: text;
  }
  input::placeholder {
    color: var(--mute);
  }
  input:focus-visible {
    outline: none;
  }
  .clear {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: var(--mute);
  }
  .clear:hover {
    color: var(--text);
    background: var(--hover);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .end {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }
  .count {
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
  .tool:hover,
  .tool.on {
    color: var(--text);
  }
  .tool:hover {
    background: var(--hover);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(164px, 1fr));
    gap: 28px 22px;
  }
  .none {
    padding: 40px 0;
    color: var(--mute);
    text-align: center;
  }
  .none button {
    color: var(--text);
    font-weight: 600;
  }
  .none button:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
</style>
