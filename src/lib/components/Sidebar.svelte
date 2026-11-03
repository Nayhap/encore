<script lang="ts">
  import type { Folder } from "$lib/backend";
  import { kindLabel } from "$lib/data";
  import { account, collectionMenu, downloads, fileIn, folderMenu, importFolder, moveInFolder, nav, needsOnline, newFolder, newPlaylist, player, playlists, saved, setCollapsed, ui, unfile, type View } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import FolderCover from "./saved/FolderCover.svelte";
  import { isMac, mod } from "$lib/platform";
  import Icon from "./Icon.svelte";

  const items: { label: string; icon: string; view: View }[] = [
    { label: "Home", icon: "home", view: { name: "home" } },
    { label: "Explore", icon: "compass", view: { name: "explore" } },
    { label: "Library", icon: "library", view: { name: "library" } },
    { label: "Downloads", icon: "download", view: { name: "downloads" } },
    { label: "History", icon: "history", view: { name: "history" } },
  ];

  // Your playlists (signed in): the shared list, reloaded when the account
  // changes and after every playlist change.
  $effect(() => {
    account.state.state;
    void playlists.load();
  });
  // Folders are Encore's own (D34), there signed out too; each account has its own.
  $effect(() => {
    account.state.state;
    void saved.load();
  });
  // Lists filed in a folder show inside it, not again below.
  const pinned = $derived.by(() => {
    const filed = saved.filed;
    return playlists.list.filter((c) => !filed.has(c.id));
  });
  const isActive = (v: View) => nav.view.name === v.name;
  const isOpen = (id: string) => nav.view.name === "collection" && nav.view.id === id;
  const isFolderOpen = (id: string) => nav.view.name === "folder" && nav.view.id === id;
  const itemsText = (n: number) => (n === 1 ? "1 item" : n ? `${n} items` : "empty");

  // The narrow rail shows covers and icons only: names move to tooltips
  // and accessible names, which the wide sidebar doesn't need.
  let compact = $state(false);
  $effect(() => {
    const mq = matchMedia("(max-width: 1000px)");
    compact = mq.matches;
    const on = () => (compact = mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  });
  const tip = (s: string) => (compact ? s : undefined);

  function addMenu(e: MouseEvent) {
    ui.openMenu(e, [
      { label: "New playlist…", icon: "plus", disabled: !account.signedIn ? "Sign in to create playlists." : needsOnline("create a playlist"), run: () => newPlaylist() },
      { label: "New folder…", icon: "folder", run: () => newFolder() },
      { label: "Import folder…", icon: "download", run: () => void importFolder() },
    ]);
  }

  // Drag a list onto a folder to file it there, within a folder to order
  // it, onto the loose lists to take it out again.
  const DRAG = "application/x-encore-list";
  let dragging = $state<{ id: string; from: Folder | null } | null>(null);
  let over = $state<string | null>(null);

  function start(e: DragEvent, id: string, from: Folder | null) {
    dragging = { id, from };
    e.dataTransfer?.setData(DRAG, id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function end() {
    dragging = null;
    over = null;
  }
  function hover(e: DragEvent, key: string) {
    if (!dragging) return;
    e.preventDefault();
    over = key;
  }
  function dropOnFolder(f: Folder, before: string | null = null) {
    const d = dragging;
    end();
    if (!d) return;
    if (d.from?.id === f.id) {
      if (before !== d.id) void moveInFolder(f, d.id, before);
    } else void fileIn(f, d.id);
  }
  function dropLoose() {
    const d = dragging;
    end();
    if (d?.from) void unfile(d.from, d.id);
  }
  const initial = $derived((account.name ?? "Y").trim()[0]?.toUpperCase() ?? "Y");
</script>

<nav class="side" aria-label="Main">
  <div class="drag" data-tauri-drag-region>
    {#if !isMac}<span class="brand" data-tauri-drag-region><img src="/icon.png" alt="" data-tauri-drag-region /><span data-tauri-drag-region>Encore</span></span>{/if}
  </div>

  <button class="search" aria-label={tip("Search")} title={tip(`Search (${mod("K")})`)} onclick={() => (ui.palette = true)}>
    <Icon name="search" size={16} />
    <span>Search</span>
    <kbd>{mod("K")}</kbd>
  </button>

  <ul class="primary">
    {#each items as it}
      <li>
        <button class="nav" class:active={isActive(it.view)} aria-current={isActive(it.view) ? "page" : undefined} aria-label={tip(it.label)} title={tip(it.label)} onclick={() => nav.go(it.view)}>
          <Icon name={it.icon} size={19} />
          <span>{it.label}</span>
          {#if it.view.name === "downloads"}
            {#if !ui.online}
              <span class="suffix" role="img" aria-label="Offline" title="You're offline. Downloaded songs still play."><Icon name="offline" size={15} /></span>
            {:else if downloads.busy && downloads.summary}
              <span class="suffix count" title="{downloads.summary.waiting} waiting to download">{downloads.summary.waiting || ""}</span>
              <span class="dot" aria-hidden="true"></span>
            {/if}
          {/if}
        </button>
      </li>
    {/each}
  </ul>

  <div class="section">
    <span>Playlists</span>
    <button class="icon-btn" aria-label="New playlist or folder" title="New playlist or folder" onclick={addMenu}><Icon name="plus" size={16} /></button>
  </div>
  <ul class="pins" class:loose-over={over === "loose"} ondragover={(e) => dragging?.from && hover(e, "loose")} ondragleave={() => over === "loose" && (over = null)} ondrop={dropLoose}>
    {#each saved.folders as f (f.id)}
      <li class="folder" class:over={over === f.id}>
        <div class="frow" role="group" ondragover={(e) => { e.stopPropagation(); hover(e, f.id); }} ondrop={(e) => { e.stopPropagation(); dropOnFolder(f); }}>
          <button
            class="pin"
            class:active={isFolderOpen(f.id)}
            aria-label={tip(`${f.name}, folder, ${itemsText(f.items.length)}`)}
            title={tip(`${f.name} · ${itemsText(f.items.length)}`)}
            onclick={() => nav.go({ name: "folder", id: f.id, title: f.name })}
            oncontextmenu={(e) => ui.openMenu(e, folderMenu(f))}
          >
            <span class="thumb"><FolderCover folder={f} radius={5} size={120} /></span>
            <span class="txt"><span class="name ell">{f.name}</span></span>
            <span class="n" aria-label={itemsText(f.items.length)}>{f.items.length || ""}</span>
          </button>
          {#if f.items.length}
            <button
              class="chev"
              aria-expanded={!f.collapsed}
              aria-label={f.collapsed ? `Show what's in ${f.name}` : `Hide what's in ${f.name}`}
              title={f.collapsed ? `Show what's in ${f.name}` : `Hide what's in ${f.name}`}
              onclick={() => setCollapsed(f, !f.collapsed)}
            >
              <Icon name="down" size={compact ? 12 : 15} />
            </button>
          {/if}
        </div>
        {#if !f.collapsed && f.items.length}
          <ul class="inner">
            {#each f.items as it (it.itemId)}
              {@const c = it.card}
              <li class:over={over === `${f.id}/${c.id}`}>
                <button
                  class="pin"
                  class:active={isOpen(c.id)}
                  draggable="true"
                  ondragstart={(e) => start(e, c.id, f)}
                  ondragend={end}
                  ondragover={(e) => { e.stopPropagation(); hover(e, `${f.id}/${c.id}`); }}
                  ondrop={(e) => { e.stopPropagation(); dropOnFolder(f, c.id); }}
                  aria-label={tip(`${c.title}, in ${f.name}`)}
                  title={tip(`${c.title} · in ${f.name}`)}
                  onclick={() => nav.go({ name: "collection", id: c.id })}
                  oncontextmenu={(e) => ui.openMenu(e, collectionMenu(c.id))}
                >
                  <span class="thumb"><Cover art={c.art} radius={5} size={120} label={c.title} /></span>
                  <span class="txt">
                    <span class="name ell">{c.title}</span>
                    <span class="sub ell">{c.subtitle || kindLabel[c.kind]}</span>
                  </span>
                  {#if player.source?.collectionId === c.id}
                    <span class="eq" class:paused={!player.playing} aria-label="Playing"><i></i><i></i><i></i></span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}
    {#each pinned as c (c.id)}
      {@const playing = player.source?.collectionId === c.id}
      <li>
        <button
          class="pin"
          class:active={isOpen(c.id)}
          draggable={saved.folders.length ? "true" : undefined}
          ondragstart={(e) => start(e, c.id, null)}
          ondragend={end}
          aria-label={tip(c.title)}
          title={tip(c.title)}
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
    <button class="account" aria-label={tip("Settings")} title={tip("Settings")} onclick={() => nav.go({ name: "settings" })}>
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

  .suffix {
    margin-left: auto;
    color: var(--mute);
  }
  .count {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  /* Narrow sidebar: the count is hidden, a dot says downloads are running. */
  .dot {
    display: none;
    position: absolute;
    top: 7px;
    right: 9px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
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

  /* Folders (D34): a row like a playlist's, then what's in it, indented
     under a hairline. */
  .frow {
    position: relative;
    display: flex;
    align-items: center;
  }
  .frow .pin {
    padding-right: 34px;
  }
  .n {
    font-size: 12px;
    color: var(--dim);
    font-variant-numeric: tabular-nums;
  }
  .chev {
    position: absolute;
    right: 4px;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    color: var(--dim);
    transition: color 0.15s, background-color 0.15s;
  }
  .chev :global(svg) {
    transition: transform 0.18s var(--ease);
  }
  .chev[aria-expanded="false"] :global(svg) {
    transform: rotate(-90deg);
  }
  .chev:hover,
  .chev:focus-visible {
    color: var(--text);
    background: var(--hover);
  }
  .frow:hover .chev {
    color: var(--mute);
  }
  .inner {
    margin: 0 0 4px 25px;
    padding-left: 6px;
    border-left: 1px solid var(--line);
  }
  .inner .thumb {
    width: 30px;
  }
  .inner .pin {
    padding: 4px 6px;
  }
  .inner .name {
    font-size: 13px;
  }
  .inner .sub {
    font-size: 11.5px;
  }
  .folder.over > .frow .pin,
  .inner li.over .pin {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 70%, transparent);
  }
  .inner li.over .pin {
    box-shadow: inset 0 2px 0 var(--accent);
    background: none;
  }
  .pins.loose-over {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
    border-radius: 9px;
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
    .suffix,
    .pin .txt,
    .account .txt,
    .account > :global(svg),
    .eq,
    .hint {
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
    .nav .dot {
      display: block;
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
    .n {
      display: none;
    }
    /* The folder's expander becomes a small tab under its cover, and what's
       in it hangs from a hairline, so the nesting reads without names. */
    .frow {
      flex-direction: column;
    }
    .frow .pin {
      padding-right: 4px;
    }
    .chev {
      position: static;
      width: 40px;
      height: 14px;
      margin: 1px 0 2px;
      border-radius: 5px;
    }
    .inner {
      margin: 0 0 6px 6px;
      padding-left: 5px;
      border-left: 2px solid var(--line);
    }
    .inner .pin {
      padding: 3px 4px;
    }
    .inner .thumb {
      width: 31px;
    }
  }
</style>
