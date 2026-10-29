<script lang="ts">
  import { call, type Page } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";

  // Any YouTube Music browse page: moods, charts, new releases, "more" links.
  let { browseId, params = null, title = "" }: { browseId: string; params?: string | null; title?: string } = $props();

  const page = new Resource<Page>(async () => {
    const p = await call((a) => a.browsePage(browseId, params));
    catalog.addPage(p);
    return p;
  });
</script>

<div class="page">
  <h1 class="page-title">{page.data?.title ?? title}</h1>
  <Status loading={page.loading && !page.data} error={page.error} onretry={() => page.reload()} empty={page.data && !page.data.shelves.length ? "This page is empty." : null} />
  {#if page.data}<Shelves shelves={page.data.shelves} />{/if}
</div>
