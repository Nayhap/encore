<script lang="ts">
  // Library → Songs: your liked songs, with Find, Sort and Select (D34).
  import type { Entry, Track } from "$lib/backend";
  import TrackTable from "$lib/components/TrackTable.svelte";
  import SelectionBar from "$lib/components/library-tools/SelectionBar.svelte";
  import Toolbar from "$lib/components/library-tools/Toolbar.svelte";
  import { ListTools } from "$lib/components/library-tools/tools.svelte";
  import Status from "$lib/components/Status.svelte";

  let { songs }: { songs: Track[] } = $props();

  // Rows of a list of songs: each is its own entry.
  const entries = $derived<Entry[]>(songs.map((t, i) => ({ entryId: `${t.id}#${i}`, track: t, setVideoId: null, videoId: t.id })));
  const tools = new ListTools(() => entries, { noun: "liked music", album: false });
  const view = $derived(tools.view);
  $effect(() => {
    if (entries.length) tools.refreshFacts();
  });
  const select = $derived(tools.selecting ? { has: (i: number) => tools.selected.has(view[i]?.entryId), toggle: (i: number, range: boolean) => view[i] && tools.toggle(view[i], range) } : undefined);
</script>

{#if tools.selecting}
  <SelectionBar {tools} />
{:else if songs.length > 1}
  <div class="list-head">
    <p>{tools.query.trim() ? (view.length === 1 ? "1 song matches" : `${view.length.toLocaleString()} songs match`) : ""}</p>
    <Toolbar {tools} />
  </div>
{/if}
{#if view.length}
  <TrackTable tracks={view.map((e) => e.track)} label="Liked music" collectionId="ytm:VLLM" {select} />
{:else}
  <Status empty={`No songs match “${tools.query.trim()}”.`} />
{/if}

<style>
  .list-head {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 40px;
    margin-bottom: 6px;
  }
  p {
    flex: 1;
    font-size: 13px;
    color: var(--mute);
  }
</style>
