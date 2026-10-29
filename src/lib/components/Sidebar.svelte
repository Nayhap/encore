<script lang="ts">
  import { call, type CollectionCard } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { kindLabel } from "$lib/data";
  import { account, collectionMenu, nav, player, ui, type View } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import { isMac, mod } from "$lib/platform";
  import Icon from "./Icon.svelte";

  const items: { label: string; icon: string; view: View }[] = [
    { label: "Home", icon: "home", view: { name: "home" } },
    { label: "Explore", icon: "compass", view: { name: "explore" } },
    { label: "Library", icon: "library", view: { name: "library" } },
  ];

  // Your playlists (signed in); refreshed when the account changes.
  let pinned = $state<CollectionCard[]>([]);
  $effect(() => {
    const s = account.state.state;
    if (s !== "signedIn") {
      pinned = [];
      return;
    }
    call((a) => a.library("playlists"))
      .then((p) => {
        catalog.addItems(p.items);
        pinned = p.items.filter((i) => i.type === "collection") as CollectionCard[];
      })
      .catch(() => (pinned = []));
  });
  const isActive = (v: View) => nav.view.name === v.name;
  const isOpen = (id: string) => nav.view.name === "collection" && nav.view.id === id;
  const initial = $derived((account.name ?? "Y").trim()[0]?.toUpperCase() ?? "Y");
</script>

<nav class="side" aria-label="Main">
  <div class="drag" data-tauri-drag-region>
    {#if !isMac}<span class="brand" data-tauri-drag-region><img src="/icon.png" alt="" data-tauri-drag-region /><span data-tauri-drag-region>Encore</span></span>{/if}
  </div>

  <button class="search" onclick={() => (ui.palette = true)}>
    <Icon name="search" size={16} />
    <span>Search</span>
    <kbd>{mod("K")}</kbd>
  </button>

  <ul class="primary">
    {#each items as it}
      <li>
        <button class="nav" class:active={isActive(it.view)} aria-current={isActive(it.view) ? "page" : undefined} onclick={() => nav.go(it.view)}>
          <Icon name={it.icon} size={19} />
          <span>{it.label}</span>
        </button>
      </li>
    {/each}
  </ul>

  <div class="section">
    <span>Playlists</span>
    <button class="icon-btn" aria-label="New playlist" title="New playlist" onclick={() => ui.say(account.signedIn ? "Create playlists on music.youtube.com for now; they show up here" : "Sign in to create playlists")}><Icon name="plus" size={16} /></button>
  </div>
  <ul class="pins">
    {#each pinned as c (c.id)}
      {@const playing = player.source?.collectionId === c.id}
      <li>
        <button
          class="pin"
          class:active={isOpen(c.id)}
          onclick={() => nav.go({ name: "collection", id: c.id })}
          oncontextmenu={(e) => ui.openMenu(e, collectionMenu(c.id))}
        >
          <span class="thumb"><Cover art={c.art} radius={5} size={120} label={c.title} /></span>
          <span class="txt">
            <span class="name ell">{c.title}</span>
            <span class="sub ell">{c.subtitle || kindLabel[c.kind]}</span>
          </span>
          {#if playing}
            <span class="eq" class:paused={!player.playing} aria-label="Playing"><i></i><i></i><i></i></span>
          {/if}
        </button>
      </li>
    {:else}
      <li class="hint">{account.signedIn ? "No playlists yet." : "Sign in to see your playlists."}</li>
    {/each}
  </ul>

  <div class="foot">
    <button class="account" onclick={() => nav.go({ name: "settings" })}>
      <span class="avatar">{initial}</span>
      <span class="txt">
        <span class="name ell">{account.name ?? (account.state.state === "expired" ? "Session expired" : account.state.state === "choosingChannel" ? "Choose a channel" : "Not signed in")}</span>
        <span class="sub">Settings</span>
      </span>
      <Icon name="settings" size={17} />
    </button>
  </div>
</nav>

<style>
  .hint {
    padding: 8px 12px;
    font-size: 12.5px;
    color: var(--dim);
  }
  .side {
    grid-area: side;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 0 10px 10px;
    background: var(--panel);
    border-right: 1px solid var(--line);
  }
  .drag {
    height: var(--top-h);
    flex: none;
    display: flex;
    align-items: center;
  }
  /* Windows and Linux have no traffic lights here; show the app's name. */
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    padding-left: 8px;
    font-family: var(--font-display);
    font-weight: 650;
    font-size: 15px;
    letter-spacing: -0.01em;
    color: var(--text);
  }
  .brand img {
    width: 22px;
    height: 22px;
    border-radius: 6px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 34px;
    margin: 0 2px 10px;
    padding: 0 8px 0 11px;
    border-radius: 9px;
    background: var(--hover);
    color: var(--mute);
    font-size: 13.5px;
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .search span {
    flex: 1;
    text-align: left;
  }
  .search:hover {
    color: var(--text);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .nav {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    height: 36px;
    padding: 0 10px;
    border-radius: 9px;
    font-weight: 550;
    color: var(--mute);
  }
  .nav:hover {
    color: var(--text);
    background: var(--hover);
  }
  .nav.active {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 9%, transparent);
  }
  .nav.active::before {
    content: "";
    position: absolute;
    left: -10px;
    top: 9px;
    bottom: 9px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--accent);
  }

  .section {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 20px 0 4px;
    padding: 0 2px 0 10px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .section .icon-btn {
    width: 26px;
    height: 26px;
  }
  .pins {
    flex: 1;
    overflow-y: auto;
    margin: 0 -4px;
    padding: 0 4px;
  }
  .pin {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 5px 6px;
    border-radius: 9px;
    text-align: left;
  }
  .pin:hover {
    background: var(--hover);
  }
  .pin.active {
    background: color-mix(in srgb, var(--text) 9%, transparent);
  }
  .thumb {
    width: 38px;
    flex: none;
  }
  .txt {
    flex: 1;
    display: grid;
    min-width: 0;
  }
  .name {
    font-size: 13.5px;
    font-weight: 550;
  }
  .sub {
    font-size: 12px;
    color: var(--mute);
  }

  .eq {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 12px;
    margin-right: 4px;
    color: var(--accent);
  }
  .eq i {
    width: 3px;
    height: 100%;
    border-radius: 1px;
    background: currentColor;
    animation: eq 0.9s ease-in-out infinite alternate;
  }
  .eq i:nth-child(2) {
    animation-delay: -0.3s;
  }
  .eq i:nth-child(3) {
    animation-delay: -0.6s;
  }
  .eq.paused i {
    animation-play-state: paused;
    height: 40%;
  }
  @keyframes eq {
    from {
      height: 25%;
    }
    to {
      height: 100%;
    }
  }

  .foot {
    padding-top: 8px;
    border-top: 1px solid var(--line);
  }
  .account {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px;
    border-radius: 9px;
    text-align: left;
    color: var(--mute);
  }
  .account:hover {
    background: var(--hover);
    color: var(--text);
  }
  .account .name {
    color: var(--text);
  }
  .avatar {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 700;
    font-size: 13px;
  }

  @media (max-width: 1000px) {
    .side {
      align-items: center;
    }
    .brand span,
    .search span,
    .search kbd,
    .nav span,
    .section span,
    .pin .txt,
    .account .txt,
    .account > :global(svg),
    .eq {
      display: none;
    }
    .search {
      width: 40px;
      justify-content: center;
      padding: 0;
    }
    .nav {
      justify-content: center;
      width: 44px;
      padding: 0;
    }
    .section {
      justify-content: center;
      padding: 0;
    }
    .pin {
      padding: 4px;
    }
    .thumb {
      width: 40px;
    }
  }
</style>
