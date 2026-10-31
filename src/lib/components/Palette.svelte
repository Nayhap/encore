<script lang="ts">
  import { call, type AnyArt, type SearchResults } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { artistLine, fmt, kindLabel } from "$lib/data";
  import { downloads, nav, needsOnline, newPlaylist, playArtist, player, ui } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import { mod, shift } from "$lib/platform";
  import Icon from "./Icon.svelte";

  // One box for finding music and for doing things. Enter plays or opens,
  // Shift+Enter puts a song next in the queue. Search is debounced (200 ms)
  // and stale answers are dropped.

  type Result = {
    key: string;
    group: string;
    title: string;
    sub: string;
    art?: AnyArt;
    round?: boolean;
    icon?: string;
    hint?: string;
    run: () => void;
    alt?: () => void;
  };

  let q = $state("");
  let sel = $state(0);
  let input: HTMLInputElement;
  let list: HTMLDivElement;
  let found = $state<SearchResults | null>(null);
  let searching = $state(false);
  let failed = $state<string | null>(null);

  const norm = (s: string) => s.toLowerCase().normalize("NFKC");
  let recent = $state<string[]>(JSON.parse(localStorage.getItem("recentSearches") ?? "[]"));
  const remember = (s: string) => {
    recent = [s, ...recent.filter((x) => x !== s)].slice(0, 5);
    localStorage.setItem("recentSearches", JSON.stringify(recent));
  };

  // Built from the current state, so each reads what it would do now.
  const downloadActions = $derived.by<Result[]>(() => {
    if (!downloads.available) return [];
    const out: Result[] = [];
    const cur = player.current;
    if (cur && !downloads.has(cur.id)) out.push({ key: "a-dl", group: "Actions", icon: "download", title: "Download this song", sub: cur.title, hint: "D", run: () => void downloads.download([cur.id]) });
    const v = nav.view;
    if (v.name === "collection" && !downloads.keptFor(v.id) && ui.online) out.push({ key: "a-dl-col", group: "Actions", icon: "download", title: "Download this playlist or album", sub: "Keep it downloaded and up to date", run: () => void downloads.keep(v.id) });
    const s = downloads.summary;
    if (s?.paused) out.push({ key: "a-dl-resume", group: "Actions", icon: "play", title: "Resume downloads", sub: "Downloads", run: () => downloads.pause(false) });
    else if (s?.waiting) out.push({ key: "a-dl-pause", group: "Actions", icon: "pause", title: "Pause downloads", sub: "Downloads", run: () => downloads.pause(true) });
    if (downloads.kept.some((k) => k.mine) && ui.online) out.push({ key: "a-dl-sync", group: "Actions", icon: "repeat", title: "Check downloads for changes", sub: "Kept playlists and albums", run: () => downloads.sync(null) });
    return out;
  });

  const actions = $derived<Result[]>([
    ...downloadActions,
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
  ]);

  let seq = 0;
  $effect(() => {
    const s = q.trim();
    failed = null;
    // Offline, only downloaded songs are searched, here (D23).
    if (!s || !ui.online) {
      found = null;
      searching = false;
      return;
    }
    searching = true;
    const my = ++seq;
    const t = setTimeout(async () => {
      try {
        const r = await call((a) => a.search(s, null));
        if (my !== seq) return;
        catalog.addSearch(r);
        if (r.top?.type === "track") catalog.addTracks([r.top]);
        found = r;
      } catch (e: any) {
        if (my === seq) failed = e?.message ?? String(e);
      } finally {
        if (my === seq) searching = false;
      }
    }, 200);
    return () => clearTimeout(t);
  });

  const results = $derived.by<Result[]>(() => {
    const s = norm(q.trim());
    if (!s) {
      return [
        ...recent.map((r) => ({ key: "r-" + r, group: "Recent searches", icon: "clock", title: r, sub: "", run: () => (q = r) })),
        ...actions,
      ];
    }
    const hit = (...xs: string[]) => xs.some((x) => norm(x).includes(s));
    const acts = actions.filter((a) => hit(a.title, a.sub));
    if (!ui.online) {
      const local = [...downloads.items.values()]
        .filter((i) => i.state === "ready" && hit(i.track.title, artistLine(i.track), i.track.album?.name ?? ""))
        .slice(0, 8)
        .map<Result>(({ track: t }) => ({
          key: t.id, group: "Downloaded songs", title: t.title, sub: `${artistLine(t)}${t.duration ? ` · ${fmt(t.duration)}` : ""}`,
          art: t.art, hint: "Play",
          run: () => player.playTracks([t], 0, { label: "Downloads", collectionId: null, radioPlaylist: null }),
          alt: () => (player.playNext(t.id), ui.say(`“${t.title}” plays next`)),
        }));
      return [...local, ...acts];
    }
    const r = found;
    if (!r) return acts;
    const songs = [...r.songs, ...r.videos.slice(0, 2)].slice(0, 6).map<Result>((t) => ({
      key: t.id, group: "Songs", title: t.title, sub: `${artistLine(t)}${t.duration ? ` · ${fmt(t.duration)}` : ""}`,
      art: t.art, hint: "Play",
      run: () => (remember(q.trim()), player.playTrack(t.id)),
      alt: () => (player.playNext(t.id), ui.say(`“${t.title}” plays next`)),
    }));
    const ars = r.artists.slice(0, 3).map<Result>((a) => ({
      key: a.id, group: "Artists", title: a.name, sub: a.subtitle || "Artist", art: a.art, round: true, hint: "Open",
      run: () => (remember(q.trim()), nav.go({ name: "artist", id: a.id })),
      alt: () => playArtist(a.id),
    }));
    const cols = [...r.albums.slice(0, 3), ...r.playlists.slice(0, 2)].map<Result>((c) => ({
      key: c.id, group: "Albums & playlists", title: c.title, sub: c.subtitle || kindLabel[c.kind], art: c.art, hint: "Open",
      run: () => (remember(q.trim()), nav.go({ name: "collection", id: c.id })),
    }));
    // The service's top result goes first, whatever its type.
    let top: Result | undefined;
    const tr = r.top;
    if (tr?.type === "artist") top = ars.find((x) => x.key === tr.id) ?? { key: tr.id, group: "", title: tr.name, sub: tr.subtitle, art: tr.art, round: true, hint: "Open", run: () => (remember(q.trim()), nav.go({ name: "artist", id: tr.id })) };
    else if (tr?.type === "track") {
      top = songs.find((x) => x.key === tr.id) ?? { key: tr.id, group: "", title: tr.title, sub: artistLine(tr), art: tr.art, hint: "Play", run: () => (remember(q.trim()), player.playTrack(tr.id)), alt: () => player.playNext(tr.id) };
    } else if (tr?.type === "collection") top = cols.find((x) => x.key === tr.id) ?? { key: tr.id, group: "", title: tr.title, sub: tr.subtitle, art: tr.art, hint: "Open", run: () => (remember(q.trim()), nav.go({ name: "collection", id: tr.id })) };
    const all = [...songs, ...ars, ...cols];
    top ??= all[0];
    const rest = all.filter((x) => x.key !== top?.key);
    return [...(top ? [{ ...top, group: "Top result" }] : []), ...rest, ...acts];
  });

  $effect(() => {
    q;
    sel = 0;
  });
  $effect(() => {
    input?.focus();
  });
  $effect(() => {
    list?.querySelector(`[data-n="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function choose(r: Result, alt = false) {
    if (alt && r.alt) r.alt();
    else r.run();
    if (!r.key.startsWith("r-")) ui.palette = false;
  }

  function key(e: KeyboardEvent) {
    if (e.key === "ArrowDown") sel = Math.min(sel + 1, results.length - 1);
    else if (e.key === "ArrowUp") sel = Math.max(sel - 1, 0);
    else if (e.key === "Enter" && results[sel]) choose(results[sel], e.shiftKey);
    else if (e.key === "Escape") ui.palette = false;
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && (ui.palette = false)}>
  <div class="box" role="dialog" aria-modal="true" aria-label="Search">
    <div class="field">
      <Icon name="search" size={19} />
      <input
        bind:this={input}
        bind:value={q}
        onkeydown={key}
        placeholder="Search songs, artists, albums, or type a command"
        spellcheck="false"
        aria-controls="palette-list"
        aria-activedescendant="pr-{sel}"
      />
      <kbd>esc</kbd>
    </div>
    <div class="list" id="palette-list" role="listbox" bind:this={list}>
      {#each results as r, i (r.key + r.group)}
        {#if i === 0 || results[i - 1].group !== r.group}
          <div class="group">{r.group}</div>
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
        {#if searching}
          <p class="empty">Searching YouTube Music…</p>
        {:else if failed}
          <p class="empty">Search failed: {failed}</p>
        {:else if !ui.online}
          <p class="empty">You're offline: only downloaded songs can be found, and none match “{q}”.</p>
        {:else}
          <p class="empty">No matches for “{q}”. Try an artist name or part of a song title.</p>
        {/if}
      {/each}
    </div>
    <footer>
      <span><kbd>↑</kbd><kbd>↓</kbd> Move</span>
      <span><kbd>↵</kbd> Play or open</span>
      <span><kbd>{shift}</kbd><kbd>↵</kbd> Play next</span>
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
  .list {
    overflow-y: auto;
    padding: 6px;
  }
  .group {
    padding: 10px 10px 4px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
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
</style>
