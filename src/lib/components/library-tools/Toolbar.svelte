<script lang="ts">
  // A list's toolbar (D34): Find songs on the left; Sort, Duplicates (when
  // there are any) and Select on the right. ⌘F / Ctrl+F goes to Find.
  import type { MenuItem } from "$lib/state.svelte";
  import { ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { mod } from "$lib/platform";
  import Glyph from "./Glyph.svelte";
  import type { SortKey } from "./logic";
  import { sortLabels, type ListTools } from "./tools.svelte";

  let { tools, duplicates = false, select = true }: { tools: ListTools; duplicates?: boolean; select?: boolean } = $props();

  let input = $state<HTMLInputElement>();

  function openFind() {
    input?.focus();
    input?.select();
  }
  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      tools.query = "";
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
    items.push({ label: "Reverse the order", icon: "swap", sep: true, run: () => tools.reverse() });
    ui.openMenu(e, items);
  }
  const sorted = $derived(tools.key !== "order" || tools.desc);
</script>

<svelte:window onkeydown={global} />

<div class="wrap">
  <div class="tools" role="group" aria-label="Library tools">
    <label class="find" class:filled={tools.query !== ""}>
      <Icon name="search" size={15} />
      <input bind:this={input} bind:value={tools.query} placeholder="Find songs" aria-label="Find songs in {tools.kind.noun}" spellcheck="false" autocomplete="off" onkeydown={key} />
      {#if tools.query}
        <button class="clear" aria-label="Clear" onclick={() => ((tools.query = ""), input?.focus())}><Icon name="x" size={13} /></button>
      {:else}
        <kbd aria-hidden="true">{mod("F")}</kbd>
      {/if}
    </label>
    <span class="grow"></span>
    <button class="tool" class:on={sorted} aria-label={sorted ? `Sorted by ${label(tools.key)}, ${way(tools.key, tools.desc)}. Change sort` : "Sort"} title={sorted ? `Sorted by ${label(tools.key).toLowerCase()}, ${way(tools.key, tools.desc)}` : "Sort"} onclick={sortMenu}>
      <Glyph name={sorted && !tools.desc && tools.key !== "order" ? "sortUp" : "sort"} />
      <span>{sorted ? label(tools.key) : "Sort"}</span>
    </button>
    {#if duplicates}
      <button class="tool" onclick={() => tools.review()} title="Exact copies, and songs that are here as both song and video" aria-label="{tools.dupCount} duplicates: review them">
        <Glyph name="dupes" />
        <span>{tools.dupCount} {tools.dupCount === 1 ? "duplicate" : "duplicates"}</span>
      </button>
    {/if}
    {#if select}
      <button class="tool" onclick={() => tools.start()} aria-label="Select songs" title="Select songs"><Glyph name="select" /><span>Select</span></button>
    {/if}
  </div>
</div>

<style>
  /* Laid out by the list's own width, not the window's: a closed panel
     gives the list room a wide window might not. */
  .wrap {
    container-type: inline-size;
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .grow {
    flex: 1;
    min-width: 8px;
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
  .tool.on {
    color: var(--accent);
  }
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 1 260px;
    min-width: 132px;
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
  kbd {
    margin-right: 4px;
    background: none;
  }
  .find:focus-within kbd {
    visibility: hidden;
  }
  .clear {
    display: grid;
    place-items: center;
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    color: var(--mute);
  }
  .clear:hover {
    color: var(--text);
    background: var(--hover);
  }
  @container (max-width: 560px) {
    kbd {
      display: none;
    }
    .tool span {
      display: none;
    }
    .tool {
      padding: 0 9px;
    }
  }
</style>
