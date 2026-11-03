<script lang="ts">
  import { onMount } from "svelte";
  import { call, type CacheInfo, type DownloadsStorage, type StatsOverview } from "$lib/backend";
  import { sizeText } from "$lib/data";
  import { downloads, listening, nav, setup, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { mb, settings } from "./shared.svelte";

  let cache = $state<CacheInfo | null>(null);
  let stored = $state<DownloadsStorage | null>(null);
  const busy = $derived(settings.busy);
  // Maintenance stays folded away until asked for.
  let showParts = $state(false);
  let showCaches = $state(false);

  onMount(async () => {
    cache = await call((a) => a.cacheInfo()).catch(() => null);
    // Measured once here: the size of the components folder.
    const rt = await call((a) => a.runtimesStatus()).catch(() => null);
    if (rt) setup.apply(rt);
  });
  // Measured on disk, and again after downloads come or go.
  $effect(() => {
    downloads.count;
    call((a) => a.downloadsStorage())
      .then((x) => (stored = x))
      .catch(() => {});
  });

  // The player components (D22).
  const rt = $derived(setup.status);
  function removeComponents() {
    ui.confirm = {
      title: "Remove player components?",
      text: `This frees about ${mb(rt?.usedBytes || rt?.diskBytes || 0)}. Streaming and new downloads need them, so they wait until you set up playback again. Downloaded songs still play.`,
      confirm: "Remove",
      danger: true,
      run: () => void settings.run("runtimes", () => setup.remove()),
    };
  }

  // The local play log (D30): how much, and since when.
  let stats = $state<StatsOverview | null>(null);
  $effect(() => {
    listening.rev;
    call((a) => a.features.stats.overview())
      .then((x) => (stats = x))
      .catch(() => {});
  });
  const since = $derived(stats?.since ? new Date(Number(stats.since) * 1000).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" }) : null);
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

  const caches = $derived<[w: "audio" | "art" | "lyrics", name: string, size: string][]>([
    ["audio", "Audio", cache ? mb(cache.audioBytes ?? 0) : "–"],
    ["art", "Artwork", cache ? mb(cache.artBytes ?? 0) : "–"],
    ["lyrics", "Lyrics", cache ? (cache.lyricsEntries === 1 ? "1 song" : `${cache.lyricsEntries ?? 0} songs`) : "–"],
  ]);
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
      <p title={stored?.location}>
        {#if downloads.summary && !downloads.available}{downloads.summary.message}
        {:else}{downloads.count === 1 ? "1 song" : `${downloads.count} songs`} · {sizeText(stored?.bytes ?? downloads.summary?.bytes)}{/if}
      </p>
    </div>
    <div class="btns">
      <button class="pill" onclick={() => nav.go({ name: "downloads" })}>Open Downloads</button>
      <button class="pill" disabled={!downloads.available || (!downloads.items.size && !downloads.kept.length && !(stored?.bytes ?? downloads.summary?.bytes))} onclick={() => downloads.removeAll()}>Remove all…</button>
    </div>
  </div>
  <div class="row">
    <div>
      <b>Player components</b>
      <p>
        {#if !rt}Checking…
        {:else if !rt.supported}{rt.message}
        {:else if rt.installing}{rt.usable ? "Updating" : "Setting up"} · {Math.floor((rt.progress?.fraction ?? 0) * 100)}%
        {:else if rt.usable}Installed{rt.usedBytes ? ` · ${mb(rt.usedBytes)}` : ""}{rt.ready ? "" : " · an update is waiting"}
        {:else}Not installed. Playback needs them, about {mb(rt.downloadBytes ?? 0)} to download.{/if}
      </p>
    </div>
    <div class="btns">
      {#if rt?.installing}
        <button class="pill" onclick={() => setup.show()}>Show progress</button>
      {:else if rt?.usable}
        <button class="pill more" aria-expanded={showParts} onclick={() => (showParts = !showParts)}>Details<Icon name={showParts ? "up" : "down"} size={14} /></button>
      {:else if rt?.supported}
        <button class="pill primary" onclick={() => setup.show()}>Set up playback</button>
      {/if}
    </div>
  </div>
  {#if showParts && rt?.usable && !rt.installing}
    <ul class="sub" aria-label="Player components">
      <li class="acts">
        <small>{rt.components.map((c) => `${c.name} ${c.installed ?? c.version}`).join(" · ")}</small>
        <button class="pill" disabled={!!busy} onclick={() => setup.install(true).then(() => setup.show())}>Reinstall</button>
        <button class="pill" disabled={!!busy} onclick={removeComponents}>Remove…</button>
      </li>
    </ul>
  {/if}
  <div class="row">
    <div>
      <b>Listening stats</b>
      <p>{stats?.plays ? `${stats.plays.toLocaleString()} ${stats.plays === 1 ? "play" : "plays"} since ${since}, kept only on this computer` : "Nothing counted yet"}</p>
    </div>
    <button class="pill" disabled={!!busy || !stats?.months.length} onclick={clearStats}>Clear…</button>
  </div>
  <div class="row">
    <div>
      <b>Caches</b>
      <p>{cache ? mb((cache.audioBytes ?? 0) + (cache.artBytes ?? 0)) : "–"} of audio, artwork and lyrics. Clearing keeps your downloads.</p>
    </div>
    <button class="pill more" aria-expanded={showCaches} onclick={() => (showCaches = !showCaches)}>Manage caches<Icon name={showCaches ? "up" : "down"} size={14} /></button>
  </div>
  {#if showCaches}
    <ul class="sub" aria-label="Caches">
      {#each caches as [w, name, size] (w)}
        <li>
          <span>{name}</span><small>{size}</small>
          <button class="pill" disabled={!!busy} onclick={() => clear(w)}>Clear</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .pill.more {
    gap: 6px;
  }
  .sub {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 8px 0;
    list-style: none;
    border-bottom: 1px solid var(--line);
  }
  .sub li {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 40px;
    padding: 0 4px 0 12px;
    border-radius: 10px;
    font-size: 13.5px;
  }
  .sub li:not(.acts):hover {
    background: var(--hover);
  }
  .sub span {
    min-width: 72px;
    font-weight: 550;
  }
  .sub small {
    flex: 1;
    font-size: 13px;
    color: var(--mute);
  }
  .sub .acts {
    gap: 8px;
  }
  .sub .pill {
    flex: none;
    height: 28px;
    font-size: 12.5px;
  }
</style>
