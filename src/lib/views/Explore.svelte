<script lang="ts">
  import { call, type Page, type PageLink } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import { nav } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";

  const page = new Resource<Page>(async () => {
    const p = await call((a) => a.explore());
    catalog.addPage(p);
    return p;
  });

  // The untitled grid of buttons at the top becomes the three doors.
  const doorShelf = $derived(page.data?.shelves.find((s) => !s.title && s.items.every((i) => i.type === "link")) ?? null);
  const doors = $derived((doorShelf?.items ?? []).filter((i) => i.type === "link") as PageLink[]);
  const rest = $derived(page.data?.shelves.filter((s) => s !== doorShelf) ?? []);
  const icon = (l: PageLink) => (l.browseId.includes("new_releases") ? "sparkle" : l.browseId.includes("charts") ? "list" : l.browseId.includes("moods") ? "compass" : "radio");
  const open = (l: PageLink) => nav.go({ name: "page", browseId: l.browseId, params: l.params, title: l.title });
</script>

<div class="page">
  <h1 class="page-title">Explore</h1>

  {#if doors.length}
    <div class="doors">
      {#each doors.slice(0, 3) as d}
        <button class="door" onclick={() => open(d)}><Icon name={icon(d)} size={22} /><span>{d.title}</span></button>
      {/each}
    </div>
  {/if}

  <Status loading={page.loading && !page.data} error={page.error} onretry={() => page.reload()} />

  <Shelves shelves={rest} />
</div>

<style>
  .doors {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    margin-top: 18px;
  }
  .door {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 64px;
    padding: 0 20px;
    border-radius: 12px;
    background: var(--hover);
    font-family: var(--font-display);
    font-size: 18px;
    font-weight: 650;
  }
  .door:hover {
    background: color-mix(in srgb, var(--text) 12%, transparent);
  }
  .door :global(svg) {
    color: var(--accent);
  }
</style>
