<script lang="ts">
  import { call, type Page } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import ShelfGrid from "$lib/components/ShelfGrid.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";

  // Any YouTube Music browse page: moods, charts, new releases, "more" links.
  let { browseId, params = null, title = "" }: { browseId: string; params?: string | null; title?: string } = $props();

  const page = new Resource<Page>(async () => {
    const p = await call((a) => a.browsePage(browseId, params));
    catalog.addPage(p);
    return p;
  });

  // A page that is one shelf of cards ("Show all" on an artist's albums,
  // singles, playlists…) is a grid, not one long carousel.
  const single = $derived.by(() => {
    const s = page.data?.shelves ?? [];
    if (s.length !== 1) return null;
    const only = s[0];
    const allLinks = only.items.every((i) => i.type === "link");
    const songList = only.layout === "list" && only.items.every((i) => i.type === "track");
    return allLinks || songList ? null : only;
  });
</script>

<div class="page">
  {#if single && title && page.data?.title && page.data.title !== title}
    <!-- "Show all" from an artist's shelf: what it lists, under whose name. -->
    <p class="kicker">{page.data.title}</p>
    <h1 class="page-title">{title}</h1>
  {:else}
    <h1 class="page-title">{page.data?.title ?? title}</h1>
  {/if}
  <Status loading={page.loading && !page.data} error={page.error} onretry={() => page.reload()} empty={page.data && !page.data.shelves.length ? "This page is empty." : null} />
  {#if single}<ShelfGrid items={single.items} />{:else if page.data}<Shelves shelves={page.data.shelves} />{/if}
</div>

<style>
  .kicker {
    font-size: 12px;
    font-weight: 650;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--mute);
    margin-bottom: 2px;
  }
</style>
