<script lang="ts">
  import { call, type Artist } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import { account, nav, playArtist, player, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";

  let { id }: { id: string } = $props();
  const res = new Resource<Artist>(async () => {
    const a = await call((x) => x.artist(id));
    catalog.addArtist(a);
    return a;
  });
  const a = $derived(res.data);
  let subscribed = $state(false);
  $effect(() => {
    subscribed = !!a?.subscribed;
  });

  async function toggleSub() {
    if (!a?.channelId) return;
    if (!account.signedIn) return ui.say("Sign in to subscribe");
    const want = !subscribed;
    subscribed = want;
    try {
      const r = await call((x) => x.setSubscribed(a.channelId!, want));
      if (r === "rejected") {
        subscribed = !want;
        ui.say("YouTube Music did not accept that change");
      } else ui.say(want ? `Subscribed to ${a.name}` : `Unsubscribed from ${a.name}`);
    } catch (e: any) {
      subscribed = !want;
      ui.say(`Could not change the subscription: ${e.message}`);
    }
  }
</script>

{#if a}
  <div class="banner" aria-hidden="true">
    <div class="banner-art"><Cover art={a.art} radius={0} size={1080} label={a.name} /></div>
  </div>
{/if}
<div class="page">
  <Status loading={res.loading && !a} error={res.error} onretry={() => res.reload()} />
  {#if a}
    <header class="hero">
      <p class="kind">Artist{a.subscribers ? ` · ${a.subscribers}` : ""}</p>
      <h1>{a.name}</h1>
      {#if a.description}<p class="blurb">{a.description}</p>{/if}
      <div class="actions">
        <button class="pill primary" disabled={!a.topSongs.length} onclick={() => player.playTracks(a.topSongs, 0, { label: a.name, collectionId: null, radioPlaylist: a.radioPlaylist })}><Icon name="play" size={16} />Play</button>
        <button class="pill" onclick={() => playArtist(id, true)}><Icon name="shuffle" size={16} />Shuffle</button>
        <button class="pill" disabled={!a.radioPlaylist} onclick={() => a.radioPlaylist && player.startRadio(a.radioPlaylist, `${a.name} radio`)}><Icon name="radio" size={16} />Radio</button>
        {#if a.channelId}
          <button class="pill subscribe" class:on={subscribed} aria-pressed={subscribed} onclick={toggleSub}>{subscribed ? "Subscribed" : "Subscribe"}</button>
        {/if}
      </div>
    </header>

    {#if a.topSongs.length}
      <h2>Top songs</h2>
      <TrackTable tracks={a.topSongs} label={a.name} showPlays />
      {#if a.topSongsMore}
        <button class="pill all" onclick={() => nav.go({ name: "collection", id: `ytm:${a.topSongsMore!.browseId}` })}>All songs</button>
      {/if}
    {/if}

    <Shelves shelves={a.shelves} />
  {/if}
</div>

<style>
  .banner {
    position: absolute;
    inset: 0 0 auto 0;
    height: 380px;
    overflow: hidden;
    mask-image: linear-gradient(#000 30%, transparent);
    opacity: 0.55;
    pointer-events: none;
  }
  .banner-art {
    width: 100%;
    transform: translateY(-30%) scale(1.1);
    filter: blur(2px) saturate(1.2);
  }
  .page {
    position: relative;
  }
  .hero {
    padding: 110px 0 30px;
    display: grid;
    gap: 10px;
  }
  .kind {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--mute);
  }
  h1 {
    font-family: var(--font-display);
    font-size: clamp(48px, 7vw, 96px);
    font-weight: 750;
    line-height: 0.95;
    letter-spacing: -0.045em;
    font-variation-settings: "wdth" 80;
  }
  .blurb {
    max-width: 56ch;
    color: var(--mute);
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
  }
  .subscribe {
    margin-left: 8px;
    box-shadow: inset 0 0 0 1px var(--line);
    background: transparent;
  }
  .subscribe.on {
    color: var(--mute);
  }
  h2 {
    font-family: var(--font-display);
    font-size: 23px;
    font-weight: 650;
    letter-spacing: -0.02em;
    margin: 10px 0 8px;
  }
  .all {
    margin-top: 10px;
  }
</style>
