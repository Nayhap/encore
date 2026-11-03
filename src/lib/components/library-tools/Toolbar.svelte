<script lang="ts">
  // Find, Sort, Select and (when there are any) Duplicates, at the end of
  // a list's header toolbar (D34). ⌘F / Ctrl+F opens Find.
  import type { MenuItem } from "$lib/state.svelte";
  import { ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Glyph from "./Glyph.svelte";
  import type { SortKey } from "./logic";
  import { sortLabels, type ListTools } from "./tools.svelte";

  let { tools, duplicates = false, select = true }: { tools: ListTools; duplicates?: boolean; select?: boolean } = $props();

  let input = $state<HTMLInputElement>();
  let open = $state(false);
  const shown = $derived(open || tools.query !== "");

  function openFind() {
    open = true;
    queueMicrotask(() => input?.focus());
  }
  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      tools.query = "";
      open = false;
      input?.blur();
    }
  }
  function global(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f" && !ui.sheet && !ui.palette) {
      e.preventDefault();
      openFind();
    }
  }

  const ways: Partial<Record<SortKey, [string, string]>> = {
    order: ["", "reversed"],
    title: ["A to Z", "Z to A"],
    artist: ["A to Z", "Z to A"],
    album: ["A to Z", "Z to A"],
    duration: ["shortest first", "longest first"],
    year: ["oldest first", "newest first"],
    plays: ["fewest first", "most first"],
    lastfm: ["fewest first", "most first"],
    youtube: ["fewest first", "most first"],
    bpm: ["slowest first", "fastest first"],
  };
  const way = (k: SortKey, desc: boolean) => ways[k]?.[desc ? 1 : 0] ?? "";
  const orderLabel = $derived(tools.kind.album ? "Album order" : tools.kind.noun === "liked music" ? "Liked order" : "Playlist order");
  const label = (k: SortKey) => (k === "order" ? orderLabel : sortLabels[k]);

  function sortMenu(e: MouseEvent) {
    const items: MenuItem[] = tools.sorts.map((k) => {
      const on = tools.key === k;
      const w = on ? way(k, tools.desc) : "";
      return { label: w ? `${label(k)}, ${w}` : label(k), icon: on ? "check" : "none", run: () => tools.sortBy(k) };
    });
    items.push({ label: "Reverse the order", icon: "swap", run: () => tools.reverse() });
    ui.openMenu(e, items);
  }
  const sorted = $derived(tools.key !== "order" || tools.desc);
</script>

<svelte:window onkeydown={global} />

<div class="tools" role="group" aria-label="Library tools">
  <label class="find" class:shown>
    <button class="lens" tabindex={shown ? -1 : 0} aria-label="Find in {tools.kind.noun}" title="Find in {tools.kind.noun} (⌘F)" onclick={openFind}><Icon name="search" size={17} /></button>
    <input bind:this={input} bind:value={tools.query} placeholder="Find in {tools.kind.noun}" aria-label="Find in {tools.kind.noun}" spellcheck="false" autocomplete="off" onkeydown={key} onblur={() => (open = false)} tabindex={shown ? 0 : -1} />
    {#if tools.query}<button class="clear" aria-label="Clear" onclick={() => ((tools.query = ""), input?.focus())}><Icon name="x" size={14} /></button>{/if}
  </label>
  <button class="tool" class:on={sorted} aria-label={sorted ? `Sorted by ${label(tools.key)}, ${way(tools.key, tools.desc)}. Change sort` : "Sort"} onclick={sortMenu}>
    <Glyph name={sorted && !tools.desc && tools.key !== "order" ? "sortUp" : "sort"} />
    <span>{sorted ? label(tools.key) : "Sort"}</span>
  </button>
  {#if duplicates}
    <button class="tool" onclick={() => tools.review()} title="Review songs that are in here more than once" aria-label="{tools.dupCount} duplicates: review them">
      <Glyph name="dupes" />
      <span>{tools.dupCount} {tools.dupCount === 1 ? "duplicate" : "duplicates"}</span>
    </button>
  {/if}
  {#if select}
    <button class="tool" onclick={() => tools.start()} aria-label="Select songs" title="Select songs"><Glyph name="select" /><span>Select</span></button>
  {/if}
</div>

<style>
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
    min-width: 0;
  }
  .tool,
  .find {
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
  .tool.on {
    color: var(--accent);
  }
  .find {
    padding: 0;
    gap: 0;
    width: 34px;
    overflow: hidden;
    transition: width 0.22s var(--ease), background-color 0.15s;
  }
  .find.shown {
    width: clamp(160px, 22vw, 240px);
    background: var(--hover);
    color: var(--text);
  }
  .lens {
    display: grid;
    place-items: center;
    flex: none;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    color: var(--mute);
  }
  .find:not(.shown) .lens:hover {
    color: var(--text);
    background: var(--hover);
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
    color: var(--dim);
  }
  input:focus-visible {
    outline: none;
  }
  .find:has(input:focus-visible) {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 70%, transparent);
  }
  .clear {
    display: grid;
    place-items: center;
    flex: none;
    width: 26px;
    height: 26px;
    margin-right: 4px;
    border-radius: 50%;
    color: var(--mute);
  }
  .clear:hover {
    color: var(--text);
    background: var(--hover);
  }
  @media (max-width: 900px) {
    .tool span {
      display: none;
    }
    .tool {
      padding: 0 9px;
    }
  }
</style>
