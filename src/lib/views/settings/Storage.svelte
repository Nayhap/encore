<script lang="ts">
  import { onMount } from "svelte";
  import { call, type CacheInfo, type DownloadsStorage, type StatsOverview } from "$lib/backend";
  import { sizeText } from "$lib/data";
  import { downloads, listening, nav, ui } from "$lib/state.svelte";
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

  // The local play log (D30): how much, and since when.
  let stats = $state<StatsOverview | null>(null);
  $effect(() => {
    listening.rev;
    call((a) => a.features.stats.overview())
      .then((x) => (stats = x))
      .catch(() => {});
  });
  const since = $derived(stats?.since ? new Date(Number(stats.since) * 1000).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" }) : null);
  function openStats() {
    localStorage.setItem("historyTab", "stats");
    nav.go({ name: "history" });
  }
  function clearStats() {
    ui.confirm = {
      title: "Clear listening stats?",
      text: "Your plays in Encore are forgotten on this computer, for this account. Your YouTube history and downloads stay as they are.",
      confirm: "Clear stats",
      danger: true,
      run: () =>
        void settings.run("stats", async () => {
          await call((a) => a.features.stats.clear());
          listening.rev++;
          ui.say("Listening stats cleared");
        }),
    };
  }

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
      <b>Listening stats</b>
      <p>{stats?.plays ? `${stats.plays.toLocaleString()} ${stats.plays === 1 ? "play" : "plays"} since ${since}` : "Nothing counted yet"} · Kept only on this computer, for your stats and sorting by your plays.</p>
    </div>
    <div class="btns">
      <button class="pill" onclick={openStats}>Open stats</button>
      <button class="pill" disabled={!!busy || !stats?.months.length} onclick={clearStats}>Clear listening stats…</button>
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
