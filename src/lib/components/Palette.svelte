<script lang="ts">
  import { call, type ArtistCard, type CollectionCard, type SearchResults, type Track } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { artistLine, fmt, kindLabel } from "$lib/data";
  import { PREFIXES, parseTime, splitScope, type Scope } from "$lib/features/powerQuery";
  import { account, downloads, isCommand, nav, needsOnline, newPlaylist, paletteItems, playArtist, player, playlists, power, ui, type PaletteItem, type PowerRecent } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import { mod, shift } from "$lib/platform";
  import Icon from "./Icon.svelte";

  // The Power Bar (D32): one box for finding music and for doing things.
  // Enter plays or opens, Shift+Enter puts a song next in the queue. A
  // first character narrows it: > commands, # your playlists and folders,
  // @ artists; "2:15" goes to that time in the song. Opened empty, it shows
  // what was last opened from it. Search is debounced (200 ms) and stale
  // answers are dropped.

  type Result = PaletteItem & { recent?: PowerRecent };

  let q = $state("");
  let scope = $state<Scope>("all");
  let sel = $state(0);
  let input: HTMLInputElement;
  let list: HTMLDivElement;
  let found = $state<SearchResults | null>(null);
  let community = $state<CollectionCard[] | null>(null);
  let searching = $state(false);
  let failed = $state<string | null>(null);

  const SCOPES: Record<Exclude<Scope, "all">, { label: string; icon: string; hint: string; placeholder: string }> = {
    commands: { label: "Commands", icon: "keyboard", hint: "Commands", placeholder: "Type a command" },
    playlists: { label: "Your playlists", icon: "list", hint: "Your playlists and folders", placeholder: "Find one of your playlists or folders" },
    artists: { label: "Artists", icon: "artist", hint: "Artists", placeholder: "Find an artist" },
  };
  const prefixOf = (s: Exclude<Scope, "all">) => Object.keys(PREFIXES).find((k) => PREFIXES[k] === s)!;

  const norm = (s: string) => s.toLowerCase().normalize("NFKC");
  const text = $derived(q.trim());

  /** A prefix typed first becomes the scope pill. */
  function oninput() {
    if (scope !== "all") return;
    const s = splitScope(q);
    if (s) {
      scope = s.scope;
      q = s.rest;
    }
  }

  function setScope(s: Scope) {
    scope = s;
    input?.focus();
  }

  // Built from the current state, so each reads what it would do now.
  const downloadActions = $derived.by<Result[]>(() => {
    if (!downloads.available) return [];
    const out: Result[] = [];
    const cur = player.current;
    if (cur && !downloads.has(cur.id)) out.push({ key: "a-dl", group: "Actions", icon: "download", title: "Download this song", sub: cur.title, hint: "D", run: () => void downloads.download([cur.id]) });
    const v = nav.view;
    if (v.name === "collection" && !downloads.keptFor(v.id)?.mine && ui.online) out.push({ key: "a-dl-col", group: "Actions", icon: "download", title: "Download this playlist or album", sub: "Keep it downloaded and up to date", run: () => void downloads.keep(v.id) });
    const s = downloads.summary;
    if (s?.paused) out.push({ key: "a-dl-resume", group: "Actions", icon: "play", title: "Resume downloads", sub: "Downloads", run: () => downloads.pause(false) });
    else if (s?.waiting) out.push({ key: "a-dl-pause", group: "Actions", icon: "pause", title: "Pause downloads", sub: "Downloads", run: () => downloads.pause(true) });
    if (downloads.kept.some((k) => k.mine) && ui.online) out.push({ key: "a-dl-sync", group: "Actions", icon: "repeat", title: "Check downloads for changes", sub: "Kept playlists and albums", run: () => downloads.sync(null) });
    return out;
  });

  // The feature modules' items (features/hooks): commands, and things such
  // as bookmarks, saved queues and folders.
  const contributed = $derived(paletteItems.items(text));

  const actions = $derived<Result[]>([
    ...downloadActions,
    ...contributed.filter(isCommand),
    { key: "a-shuffle", group: "Actions", icon: "shuffle", title: player.shuffle ? "Turn shuffle off" : "Turn shuffle on", sub: "Playback", hint: "S", run: () => player.toggleShuffle() },
    { key: "a-repeat", group: "Actions", icon: "repeat", title: "Change repeat mode", sub: "Off, queue, or this song", hint: "R", run: () => player.cycleRepeat() },
    { key: "a-lyrics", group: "Actions", icon: "lyrics", title: "Open full-screen lyrics", sub: "Playback", hint: "F", run: () => (ui.theater = true) },
    { key: "a-stage", group: "Actions", icon: "stage", title: ui.stageOpen ? "Hide now playing panel" : "Show now playing panel", sub: "Layout", hint: mod("\\"), run: () => (ui.stageOpen = !ui.stageOpen) },
    { key: "a-theme", group: "Actions", icon: "moon", title: "Change theme", sub: "System, dark or light", run: () => nav.go({ name: "settings" }) },
    { key: "a-newpl", group: "Actions", icon: "plus", title: "New playlist…", sub: "Create a playlist", run: () => { const why = needsOnline("create a playlist"); if (why) ui.say(why); else newPlaylist(); } },
    { key: "a-lib", group: "Go to", icon: "library", title: "Library", sub: "Playlists, albums, artists", run: () => nav.go({ name: "library" }) },
    { key: "a-dls", group: "Go to", icon: "download", title: "Downloads", sub: "Songs on this computer", run: () => nav.go({ name: "downloads" }) },
    { key: "a-hist", group: "Go to", icon: "history", title: "History", sub: "What you've played", run: () => nav.go({ name: "history" }) },
    { key: "a-exp", group: "Go to", icon: "compass", title: "Explore", sub: "New releases, charts, moods", run: () => nav.go({ name: "explore" }) },
    { key: "a-set", group: "Go to", icon: "settings", title: "Settings", sub: "Account, appearance, playback, connections", run: () => nav.go({ name: "settings" }) },
  ]);

  let seq = 0;
  $effect(() => {
    const s = text;
    const sc = scope;
    failed = null;
    community = null;
    // Offline, only downloaded songs are searched, here (D23). Commands
    // and your playlists are local.
    if (!s || !ui.online || sc === "commands" || sc === "playlists" || parseTime(s) != null) {
      found = null;
      searching = false;
      return;
    }
    searching = true;
    const my = ++seq;
    const t = setTimeout(async () => {
      try {
        const r = await call((a) => a.search(s, sc === "artists" ? "artists" : null));
        if (my !== seq) return;
        catalog.addSearch(r);
        if (r.top?.type === "track") catalog.addTracks([r.top]);
        found = r;
        // Then community playlists, as a group of their own.
        if (sc === "all" && s.length >= 3) {
          const c = await call((a) => a.search(s, "communityPlaylists")).catch(() => null);
          if (my === seq && c) {
            catalog.addSearch(c);
            community = c.playlists.slice(0, 3);
          }
        }
      } catch (e: any) {
        if (my === seq) failed = e?.message ?? String(e);
      } finally {
        if (my === seq) searching = false;
      }
    }, 200);
    return () => clearTimeout(t);
  });

  // ---------- result rows ----------

  const trackRow = (t: Track, group: string): Result => ({
    key: t.id, group, title: t.title, sub: `${artistLine(t)}${t.duration ? ` · ${fmt(t.duration)}` : ""}`, art: t.art, hint: "Play",
    run: () => player.playTrack(t.id),
    alt: () => (player.playNext(t.id), ui.say(`“${t.title}” plays next`)),
    recent: { kind: "track", id: t.id, title: t.title, sub: artistLine(t), art: t.art, track: t },
  });
  const artistRow = (a: ArtistCard, group: string): Result => ({
    key: a.id, group, title: a.name, sub: a.subtitle || "Artist", art: a.art, round: true, hint: "Open",
    run: () => nav.go({ name: "artist", id: a.id }),
    alt: () => playArtist(a.id),
    recent: { kind: "artist", id: a.id, title: a.name, sub: "Artist", art: a.art, round: true },
  });
  const collectionRow = (c: CollectionCard, group: string): Result => ({
    key: c.id, group, title: c.title, sub: c.subtitle || kindLabel[c.kind], art: c.art, hint: "Open",
    run: () => nav.go({ name: "collection", id: c.id }),
    alt: () => player.playCollection(c.id),
    recent: { kind: "collection", id: c.id, title: c.title, sub: c.subtitle || kindLabel[c.kind], art: c.art },
  });
  const recentRow = (r: PowerRecent): Result => {
    if (r.kind === "track" && r.track) return { ...trackRow(r.track, "Recent"), key: "r-" + r.id };
    const run = () => (r.kind === "artist" ? nav.go({ name: "artist", id: r.id }) : nav.go({ name: "collection", id: r.id }));
    return { key: "r-" + r.id, group: "Recent", title: r.title, sub: r.sub, art: r.art, round: r.round, hint: "Open", run, recent: r };
  };

  /** "Go to 2:15" in the song that's playing. */
  const seekRow = $derived.by<Result | null>(() => {
    const t = parseTime(text);
    const cur = player.current;
    if (t == null || !cur) return null;
    const d = player.duration ?? cur.duration ?? 0;
    if (d && t >= d) return { key: "seek", group: "Playback", icon: "clock", title: `Go to ${fmt(t)}`, sub: `“${cur.title}” is ${fmt(d)} long`, run: () => ui.say(`“${cur.title}” is only ${fmt(d)} long`) };
    return { key: "seek", group: "Playback", icon: "clock", title: `Go to ${fmt(t)}`, sub: cur.title, hint: "Go", run: () => player.seek(t) };
  });

  const yourPlaylists = $derived(playlists.list.map((c) => collectionRow(c, "Your playlists")));
  const things = $derived(contributed.filter((i) => !isCommand(i)) as Result[]);

  const results = $derived.by<Result[]>(() => {
    const s = norm(text);
    const hit = (...xs: (string | undefined)[]) => !s || xs.some((x) => x && norm(x).includes(s));

    if (scope === "commands") return actions.filter((a) => hit(a.title, a.sub));
    if (scope === "playlists") {
      const folders = things.filter((i) => i.group === "Folders" && hit(i.title, i.sub));
      return [...folders, ...yourPlaylists.filter((p) => hit(p.title, p.sub)).slice(0, 20)];
    }
    if (scope === "artists") return (found?.artists ?? []).slice(0, 8).map((a) => artistRow(a, "Artists"));

    if (!s) return [...power.recent.map(recentRow), ...actions];
    const acts = actions.filter((a) => hit(a.title, a.sub));
    const mine = [...things.filter((i) => hit(i.title, i.sub)), ...yourPlaylists.filter((p) => hit(p.title)).slice(0, 3)];
    const seek = seekRow ? [seekRow] : [];
    if (!ui.online) {
      const local = [...downloads.items.values()]
        .filter((i) => i.state === "ready" && hit(i.track.title, artistLine(i.track), i.track.album?.name ?? ""))
        .slice(0, 8)
        .map<Result>(({ track: t }) => ({
          ...trackRow(t, "Downloaded songs"),
          run: () => player.playTracks([t], 0, { label: "Downloads", collectionId: null, radioPlaylist: null }),
        }));
      return [...seek, ...local, ...mine, ...acts];
    }
    const r = found;
    if (!r) return [...seek, ...mine, ...acts];
    const songs = [...r.songs, ...r.videos.slice(0, 2)].slice(0, 6).map((t) => trackRow(t, "Songs"));
    const ars = r.artists.slice(0, 3).map((a) => artistRow(a, "Artists"));
    const albums = r.albums.slice(0, 3).map((c) => collectionRow(c, "Albums"));
    // Community playlists come from their own search; until then (or if it
    // fails) the playlists of the first answer stand in.
    const cps = (community ?? r.playlists.slice(0, 2)).map((c) => collectionRow(c, community ? "Community playlists" : "Playlists"));
    // The service's top result goes first, whatever its type.
    let top: Result | undefined;
    const tr = r.top;
    if (tr?.type === "artist") top = ars.find((x) => x.key === tr.id) ?? artistRow(tr, "");
    else if (tr?.type === "track") top = songs.find((x) => x.key === tr.id) ?? trackRow(tr, "");
    else if (tr?.type === "collection") top = [...albums, ...cps].find((x) => x.key === tr.id) ?? collectionRow(tr, "");
    const all = [...songs, ...ars, ...albums, ...cps];
    top ??= all[0];
    const rest = all.filter((x) => x.key !== top?.key);
    return [...seek, ...(top ? [{ ...top, group: "Top result" }] : []), ...rest, ...mine, ...acts];
  });

  $effect(() => {
    q;
    scope;
    sel = 0;
  });
  $effect(() => {
    input?.focus();
  });
  $effect(() => {
    list?.querySelector(`[data-n="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function choose(r: Result, alt = false) {
    if (r.recent) {
      if (r.recent.track) catalog.addTracks([r.recent.track]);
      power.remember(r.recent);
    }
    if (alt && r.alt) r.alt();
    else r.run();
    ui.palette = false;
  }

  function key(e: KeyboardEvent) {
    if (e.key === "ArrowDown") sel = Math.min(sel + 1, results.length - 1);
    else if (e.key === "ArrowUp") sel = Math.max(sel - 1, 0);
    else if (e.key === "Enter" && results[sel]) choose(results[sel], e.shiftKey);
    else if (e.key === "Escape") {
      if (scope !== "all") scope = "all";
      else ui.palette = false;
    } else if (e.key === "Backspace" && !q && scope !== "all") scope = "all";
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  const placeholder = $derived(scope === "all" ? "Search songs, artists, albums, or type a command" : SCOPES[scope].placeholder);
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (ui.palette = false)}>
  <div class="box" role="dialog" aria-modal="true" aria-label="Search">
    <div class="field">
      <Icon name="search" size={19} />
      {#if scope !== "all"}
        <button class="scope" onclick={() => setScope("all")} aria-label="{SCOPES[scope].label}: back to all results">
          <Icon name={SCOPES[scope].icon} size={14} />{SCOPES[scope].label}<Icon name="x" size={12} />
        </button>
      {/if}
      <input
        bind:this={input}
        bind:value={q}
        oninput={oninput}
        onkeydown={key}
        {placeholder}
        spellcheck="false"
        aria-controls="palette-list"
        aria-activedescendant="pr-{sel}"
      />
      <kbd>esc</kbd>
    </div>
    {#if scope === "all" && !text}
      <div class="scopes" role="group" aria-label="Narrow the search">
        {#each Object.entries(SCOPES) as [s, v]}
          <button onclick={() => setScope(s as Scope)}><kbd>{prefixOf(s as Exclude<Scope, "all">)}</kbd>{v.hint}</button>
        {/each}
        {#if player.current}<span class="tip"><kbd>2:15</kbd> goes to that time</span>{/if}
      </div>
    {/if}
    <div class="list" id="palette-list" role="listbox" bind:this={list}>
      {#each results as r, i (r.key + r.group)}
        {#if i === 0 || results[i - 1].group !== r.group}
          <div class="group">
            {r.group}
            {#if r.group === "Recent" && i === 0}<button class="clear" onclick={() => power.forget()}>Clear</button>{/if}
          </div>
        {/if}
        <div
          class="item"
          class:sel={i === sel}
          class:top={r.group === "Top result"}
          id="pr-{i}"
          data-n={i}
          role="option"
          tabindex="-1"
          aria-selected={i === sel}
          onmousemove={() => (sel = i)}
          onclick={() => choose(r)}
          onkeydown={() => {}}
        >
          <span class="lead" class:round={r.round}>
            {#if r.art !== undefined}<Cover art={r.art} round={r.round} radius={5} size={120} label={r.title} />{:else}<Icon name={r.icon ?? "search"} size={17} />{/if}
          </span>
          <span class="txt">
            <span class="t ell">{r.title}</span>
            {#if r.sub}<span class="s ell">{r.sub}</span>{/if}
          </span>
          {#if r.hint && i === sel}<span class="hint">{r.hint} <kbd>↵</kbd></span>{/if}
        </div>
      {:else}
        {#if scope === "commands"}
          <p class="empty">No commands match “{text}”.</p>
        {:else if scope === "playlists"}
          <p class="empty">{account.signedIn ? (text ? `None of your playlists or folders match “${text}”.` : "Your playlists and folders show here.") : "Sign in to find your playlists here."}</p>
        {:else if scope === "artists" && !ui.online}
          <p class="empty">You're offline. Finding artists needs YouTube Music.</p>
        {:else if scope === "artists" && !text}
          <p class="empty">Type an artist's name.</p>
        {:else if searching}
          <p class="empty">Searching YouTube Music…</p>
        {:else if failed}
          <p class="empty">Search failed: {failed}</p>
        {:else if !ui.online}
          <p class="empty">You're offline: only downloaded songs can be found, and none match “{text}”.</p>
        {:else}
          <p class="empty">No matches for “{text}”. Try an artist name or part of a song title.</p>
        {/if}
      {/each}
    </div>
    <footer>
      <span><kbd>↑</kbd><kbd>↓</kbd> Move</span>
      <span><kbd>↵</kbd> Play or open</span>
      <span><kbd>{shift}</kbd><kbd>↵</kbd> Play next</span>
      {#if scope !== "all"}<span class="end"><kbd>⌫</kbd> All results</span>{/if}
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    background: rgb(0 0 0 / 0.35);
    animation: fade 0.15s;
  }
  .box {
    width: min(640px, calc(100vw - 48px));
    max-height: 68vh;
    display: flex;
    flex-direction: column;
    border-radius: 16px;
    background: var(--raise);
    box-shadow: var(--shadow), 0 40px 120px -20px rgb(0 0 0 / 0.5);
    overflow: hidden;
    animation: rise 0.2s var(--ease);
  }
  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    height: 56px;
    border-bottom: 1px solid var(--line);
    color: var(--mute);
  }
  .field input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    font-size: 17px;
    color: var(--text);
    user-select: text;
  }
  .field input::placeholder {
    color: var(--dim);
  }
  .scope {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px 0 10px;
    margin-right: -4px;
    border-radius: 99px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    background: var(--accent-soft);
    color: var(--accent);
    animation: pop 0.14s var(--ease);
  }
  .scope:hover {
    background: color-mix(in srgb, var(--accent) 24%, transparent);
  }
  .scopes {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 10px 14px 4px;
  }
  .scopes button {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 11px 0 6px;
    border-radius: 99px;
    font-size: 12.5px;
    font-weight: 550;
    color: var(--mute);
    background: var(--hover);
  }
  .scopes button:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 10%, transparent);
  }
  .scopes kbd {
    min-width: 18px;
    text-align: center;
  }
  .tip {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--dim);
  }
  .list {
    overflow-y: auto;
    padding: 6px;
  }
  .group {
    display: flex;
    justify-content: space-between;
    padding: 10px 10px 4px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .clear {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
    color: var(--dim);
  }
  .clear:hover {
    color: var(--text);
  }
  .item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 10px;
    border-radius: 10px;
  }
  .item.sel {
    background: color-mix(in srgb, var(--text) 9%, transparent);
  }
  .lead {
    width: 40px;
    height: 40px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 6px;
    background: var(--hover);
    color: var(--mute);
  }
  .item.top .lead {
    width: 52px;
    height: 52px;
  }
  .txt {
    flex: 1;
    display: grid;
    min-width: 0;
  }
  .t {
    font-weight: 600;
  }
  .item.top .t {
    font-family: var(--font-display);
    font-size: 18px;
  }
  .s {
    font-size: 12.5px;
    color: var(--mute);
  }
  .hint {
    font-size: 12px;
    color: var(--mute);
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .empty {
    padding: 24px 12px;
    color: var(--mute);
  }
  footer {
    display: flex;
    gap: 18px;
    padding: 9px 16px;
    border-top: 1px solid var(--line);
    font-size: 12px;
    color: var(--mute);
  }
  footer span {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  footer .end {
    margin-left: auto;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.98);
    }
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.9);
    }
  }
</style>
