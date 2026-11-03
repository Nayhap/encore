<script lang="ts">
  import { call, type Artist, type Shelf } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { Resource } from "$lib/load.svelte";
  import { account, artistMenuItems, nav, playArtist, player, ui, wikiFor } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Shelves from "$lib/components/Shelves.svelte";
  import Status from "$lib/components/Status.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";
  import Discography from "$lib/components/library-tools/Discography.svelte";
  import WikiBlurb from "$lib/components/WikiBlurb.svelte";

  let { id }: { id: string } = $props();
  const res = new Resource<Artist>(async () => {
    const a = await call((x) => x.artist(id));
    catalog.addArtist(a);
    return a;
  });
  const a = $derived(res.data);
  // The ⋯ menu shows once a feature puts something in it (bookmark the
  // artist, don't play them).
  const menu = $derived(a ? artistMenuItems.items(id) : []);
  // The release shelves' own links (`MPAD…` with their params): the
  // Discography tab reads every release behind them (D35).
  const releaseLinks = $derived(a ? a.shelves.flatMap((s) => (s.more?.browseId.startsWith("MPAD") ? [s.more] : [])) : []);
  type Tab = "overview" | "discography";
  let tab = $state<Tab>("overview");
  // Discography is a record shelf: the hero shrinks to one row and the bio
  // waits behind About.
  const disco = $derived(tab === "discography" && releaseLinks.length > 0);
  let about = $state(false);
  const hasBio = $derived(!!a && (!!a.description || !!wikiFor(id, a.name)));
  const TABS: [Tab, string][] = [
    ["overview", "Overview"],
    ["discography", "Discography"],
  ];
  // The tabs get a bar behind them only once they stick under the top bar.
  let sentinel = $state<HTMLElement>();
  let stuck = $state(false);
  $effect(() => {
    const el = sentinel;
    const sc = el?.closest<HTMLElement>(".scroll");
    if (!el || !sc) return;
    const check = () => (stuck = el.getBoundingClientRect().top < sc.getBoundingClientRect().top + parseFloat(getComputedStyle(sc).paddingTop) + 1);
    check();
    sc.addEventListener("scroll", check, { passive: true });
    return () => sc.removeEventListener("scroll", check);
  });
  // Tabs as in WAI-ARIA: arrow keys move between them and select.
  function tabKey(e: KeyboardEvent) {
    const i = TABS.findIndex(([t]) => t === tab);
    const j = e.key === "ArrowRight" ? i + 1 : e.key === "ArrowLeft" ? i - 1 : e.key === "Home" ? 0 : e.key === "End" ? TABS.length - 1 : null;
    if (j == null) return;
    e.preventDefault();
    tab = TABS[(j + TABS.length) % TABS.length][0];
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`#tab-${tab}`)?.focus();
  }

  // "Playlists with <artist>": what YouTube Music's community-playlist
  // search finds for the name (D35). Named for what it is, a search.
  let community = $state<Shelf | null>(null);
  $effect(() => {
    const name = a?.name;
    if (!name || !ui.online) return;
    let live = true;
    call((x) => x.search(name, "communityPlaylists"))
      .then((r) => {
        if (!live || !r.playlists.length) return;
        catalog.addItems(r.playlists.map((c) => ({ type: "collection" as const, ...c })));
        community = { title: `Playlists with ${name}`, subtitle: "From listeners", layout: "carousel", items: r.playlists.slice(0, 6).map((c) => ({ type: "collection" as const, ...c })), more: null };
      })
      .catch(() => {});
    return () => (live = false);
  });

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
  <div class="banner" class:low={disco} aria-hidden="true">
    <div class="banner-art"><Cover art={a.art} radius={0} size={1080} label={a.name} /></div>
  </div>
{/if}
<div class="page">
  <Status loading={res.loading && !a} error={res.error} onretry={() => res.reload()} />
  {#if a}
    {#snippet actions()}
      <button class="pill primary" disabled={!a.topSongs.length} onclick={() => player.playTracks(a.topSongs, 0, { label: a.name, collectionId: null, radioPlaylist: a.radioPlaylist })}><Icon name="play" size={16} />Play</button>
      <button class="pill" onclick={() => playArtist(id, true)}><Icon name="shuffle" size={16} />Shuffle</button>
      {#if !disco}
        <button class="pill" disabled={!a.radioPlaylist} onclick={() => a.radioPlaylist && player.startRadio(a.radioPlaylist, `${a.name} radio`)}><Icon name="radio" size={16} />Radio</button>
      {/if}
      {#if a.channelId}
        <button class="pill subscribe" class:on={subscribed} aria-pressed={subscribed} onclick={toggleSub}>{subscribed ? "Subscribed" : "Subscribe"}</button>
      {/if}
      {#if menu.length}
        <button class="icon-btn big" aria-label="More actions" onclick={(e) => ui.openMenu(e, artistMenuItems.items(id))}><Icon name="more" size={19} /></button>
      {/if}
    {/snippet}

    {#if disco}
      <header class="hero compact">
        <div class="avatar"><Cover art={a.art} radius={999} size={120} label={a.name} /></div>
        <div class="who">
          <h1>{a.name}</h1>
          {#if a.subscribers || hasBio}
            <p class="kind">
              {#if a.subscribers}<span>{a.subscribers}</span>{/if}
              {#if hasBio}<button class="about" aria-expanded={about} onclick={() => (about = !about)}>About</button>{/if}
            </p>
          {/if}
        </div>
        <div class="actions">{@render actions()}</div>
      </header>
      {#if about}
        <div class="bio">{#if a.description}<p class="blurb">{a.description}</p>{:else}<WikiBlurb {id} name={a.name} />{/if}</div>
      {/if}
    {:else}
      <header class="hero">
        <p class="kind">Artist{a.subscribers ? ` · ${a.subscribers}` : ""}</p>
        <h1>{a.name}</h1>
        {#if a.description}<p class="blurb">{a.description}</p>{:else}<WikiBlurb {id} name={a.name} />{/if}
        <div class="actions">{@render actions()}</div>
      </header>
    {/if}

    {#if releaseLinks.length}
      <div class="sentinel" bind:this={sentinel}></div>
      <div class="tabs" class:stuck role="tablist" aria-label={a.name} tabindex="-1" onkeydown={tabKey}>
        {#each TABS as [t, label] (t)}
          <button role="tab" id="tab-{t}" aria-selected={tab === t} aria-controls="panel-{t}" tabindex={tab === t ? 0 : -1} onclick={() => (tab = t)}>{label}</button>
        {/each}
      </div>
    {/if}

    <div role={releaseLinks.length ? "tabpanel" : undefined} id="panel-{disco ? 'discography' : 'overview'}" aria-labelledby={releaseLinks.length ? `tab-${disco ? "discography" : "overview"}` : undefined}>
      {#if disco}
        <Discography links={releaseLinks} />
      {:else}
        {#if a.topSongs.length}
          <h2>Top songs</h2>
          <TrackTable tracks={a.topSongs} label={a.name} showPlays />
          {#if a.topSongsMore}
            <button class="pill all" onclick={() => nav.go({ name: "collection", id: `ytm:${a.topSongsMore!.browseId}`, params: a.topSongsMore!.params })}>All songs</button>
          {/if}
        {/if}
        {#if community}<Shelves shelves={[community]} />{/if}
        <Shelves shelves={a.shelves} />
      {/if}
    </div>
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
    transition: height 0.3s var(--ease), opacity 0.3s;
  }
  .banner.low {
    height: 200px;
    opacity: 0.35;
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
  .big {
    width: 38px;
    height: 38px;
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
  /* Discography: identity and actions on one row. */
  .hero.compact {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 44px 0 22px;
  }
  .avatar {
    width: 64px;
    flex: none;
    border-radius: 50%;
    box-shadow: 0 10px 24px -12px rgb(0 0 0 / 0.6);
  }
  .who {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 3px;
  }
  .compact h1 {
    font-size: 34px;
    line-height: 1.05;
    letter-spacing: -0.03em;
    font-variation-settings: "wdth" 85;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .compact .kind {
    display: flex;
    gap: 6px;
  }
  .compact .kind > * + *::before {
    content: "·";
    margin-right: 6px;
    color: var(--mute);
  }
  .about {
    font: inherit;
    color: var(--mute);
  }
  .about:hover,
  .about[aria-expanded="true"] {
    color: var(--text);
  }
  .compact .actions {
    margin: 0;
    flex: none;
  }
  .compact .subscribe {
    margin-left: 0;
  }
  .bio {
    margin: -8px 0 18px 80px;
  }
  @media (max-width: 900px) {
    .hero.compact {
      flex-wrap: wrap;
    }
    .compact .actions {
      width: 100%;
    }
  }
  /* The tabs stay in reach while the page scrolls, under the top bar (the
     scroller's padding already keeps sticky things below it). */
  .tabs {
    position: sticky;
    top: 0;
    z-index: 3;
    isolation: isolate;
    display: flex;
    gap: 22px;
    height: 46px;
    margin: -6px -32px 22px;
    padding: 0 32px;
    transition: background-color 0.2s;
  }
  /* The hairline spans the content, and the whole pane once stuck. */
  .tabs::before {
    content: "";
    position: absolute;
    left: 32px;
    right: 32px;
    bottom: 0;
    height: 1px;
    background: var(--line);
    transition: left 0.2s, right 0.2s;
  }
  .tabs.stuck::before {
    left: 0;
    right: 0;
  }
  .sentinel {
    height: 0;
  }
  .tabs.stuck {
    background: var(--ink);
  }
  .tabs button {
    position: relative;
    padding: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--mute);
  }
  .tabs button:focus-visible {
    outline-offset: -6px;
    border-radius: 12px;
  }
  .tabs button:hover {
    color: var(--text);
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
  }
  .tabs button[aria-selected="true"]::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
</style>
