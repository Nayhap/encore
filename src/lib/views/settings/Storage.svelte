<script lang="ts">
  import { onMount } from "svelte";
  import { call, type CacheInfo, type DownloadsStorage } from "$lib/backend";
  import { sizeText } from "$lib/data";
  import { downloads, nav } from "$lib/state.svelte";
  import { mb, settings } from "./shared.svelte";

  let cache = $state<CacheInfo | null>(null);
  let stored = $state<DownloadsStorage | null>(null);
  const busy = $derived(settings.busy);

  onMount(async () => {
    cache = await call((a) => a.cacheInfo()).catch(() => null);
  });
  // Measured on disk, and again after downloads come or go.
  $effect(() => {
    downloads.count;
    call((a) => a.downloadsStorage())
      .then((x) => (stored = x))
      .catch(() => {});
  });

  const clear = (w: "audio" | "art" | "lyrics") =>
    settings.run("cache", async () => {
      await call((a) => a.cacheClear(w));
      cache = await call((a) => a.cacheInfo());
    });
</script>

<section>
  <h2>Storage</h2>
  <div class="row">
    <div>
      <b>Downloads</b>
      <p>
        {#if downloads.summary && !downloads.available}{downloads.summary.message}
        {:else}{downloads.count === 1 ? "1 song" : `${downloads.count} songs`} · {sizeText(stored?.bytes ?? downloads.summary?.bytes)} on this computer{#if stored} · {stored.location}{/if}{/if}
      </p>
    </div>
    <div class="btns">
      <button class="pill" onclick={() => nav.go({ name: "downloads" })}>Open Downloads</button>
      <button class="pill" disabled={!downloads.available || (!downloads.items.size && !downloads.kept.length && !(stored?.bytes ?? downloads.summary?.bytes))} onclick={() => downloads.removeAll()}>Remove all downloads…</button>
    </div>
  </div>
  <div class="row">
    <div>
      <b>Caches</b>
      <p>Audio {cache ? mb(cache.audioBytes ?? 0) : "–"} · Artwork {cache ? mb(cache.artBytes ?? 0) : "–"} · Lyrics {cache?.lyricsEntries ?? "–"} songs. Clearing them keeps your downloads.</p>
    </div>
    <div class="btns">
      <button class="pill" disabled={!!busy} onclick={() => clear("audio")}>Clear audio</button>
      <button class="pill" disabled={!!busy} onclick={() => clear("art")}>Clear artwork</button>
      <button class="pill" disabled={!!busy} onclick={() => clear("lyrics")}>Clear lyrics</button>
    </div>
  </div>
</section>
