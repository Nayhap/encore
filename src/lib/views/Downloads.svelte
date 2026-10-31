<script lang="ts">
  import { call, type KeptCollection, type Track } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { artistLine, kindLabel, sizeText } from "$lib/data";
  import { downloads, keptNote, nav, player, ui, type MenuItem } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Status from "$lib/components/Status.svelte";
  import TrackTable from "$lib/components/TrackTable.svelte";

  // Everything kept on this computer (D23): what is downloading and why it
  // waits, the songs, and the playlists and albums kept up to date.
  type Tab = "Songs" | "Playlists and albums";
  let tab = $state<Tab>("Songs");

  const all = $derived([...downloads.items.values()]);
  const ready = $derived(all.filter((i) => i.state === "ready").sort((a, b) => (b.added ?? 0) - (a.added ?? 0)));
  const waiting = $derived(all.filter((i) => i.state === "queued" || i.state === "downloading").sort((a, b) => (a.added ?? 0) - (b.added ?? 0)));
  const failed = $derived(all.filter((i) => i.state === "failed"));
  const songs = $derived(ready.map((i) => i.track));
  const s = $derived(downloads.summary);
  const active = $derived(s?.active ? (downloads.items.get(s.active.trackId)?.track ?? catalog.track(s.active.trackId)) : undefined);
  const pct = $derived(s?.active ? downloads.progress(s.active.trackId) : null);
  // Kept collections, this account's first, one row per collection.
  const kept = $derived(
    [...downloads.kept].sort((a, b) => Number(b.mine) - Number(a.mine)).filter((k, i, xs) => xs.findIndex((x) => x.card.id === k.card.id) === i),
  );

  const n = (x: number, one: string, many: string) => (x === 1 ? `1 ${one}` : `${x} ${many}`);
  const activity = $derived.by(() => {
    if (!s) return null;
    if (s.paused) return s.waiting ? `Downloads are paused · ${n(s.waiting, "song", "songs")} waiting` : "Downloads are paused";
    // Offline is said once, above.
    if (s.wait === "offline") return s.waiting ? `${n(s.waiting, "song", "songs")} waiting for a connection` : null;
    if (s.message && (s.waiting || s.syncing.length)) return s.message;
    if (active) return `Downloading “${active.title}”${s.waiting > 1 ? ` · ${s.waiting - 1} more waiting` : ""}`;
    if (s.syncing.length) return "Checking for changes…";
    if (s.waiting) return `${n(s.waiting, "song", "songs")} waiting`;
    return null;
  });

  function retryOne(t: Track) {
    void downloads.download([t.id]);
  }

  function keptMenu(e: MouseEvent, k: KeptCollection) {
    const items: MenuItem[] = [
      { label: "Open", icon: "list", run: () => nav.go({ name: "collection", id: k.card.id }) },
      { label: "Check for changes", icon: "repeat", disabled: !ui.online ? "You're offline. Connect to the internet to check for changes." : k.mine ? undefined : "Only the account that downloaded it can check it for changes.", run: () => downloads.sync(k.card.id) },
      { label: k.paused ? "Resume downloading" : "Pause downloading", icon: k.paused ? "play" : "pause", run: () => downloads.pauseKept(k.card.id, !k.paused) },
      { label: "Remove download…", icon: "trash", danger: true, run: () => downloads.unkeep(k.card.id) },
    ];
    ui.openMenu(e, items);
  }

  function shuffle() {
    player.playTracks(songs, Math.floor(Math.random() * songs.length), { label: "Downloads", collectionId: null, radioPlaylist: null }, true);
  }

  async function checkConnection() {
    const on = await call((a) => a.connectivityCheck()).catch(() => false);
    if (!on) ui.say("Still offline. Encore keeps checking, and downloads continue once it's back.");
  }
</script>

<div class="page">
  <h1 class="page-title">Downloads</h1>

  {#if s && !s.available}
    <Status empty={s.message ?? "Downloads aren't available on this computer."} />
  {:else}
    <p class="lede">
      {n(downloads.count, "song", "songs")} · {sizeText(s?.bytes)} on this computer
    </p>

    {#if !ui.online}
      <div class="notice" role="status">
        <Icon name="offline" size={18} />
        <div><b>You're offline.</b> <span>Your downloaded songs play as usual.</span></div>
        <button class="pill" onclick={checkConnection}>Check again</button>
      </div>
    {/if}

    {#if activity}
      <div class="activity" role="status" aria-live="polite">
        {#if active}<span class="thumb"><Cover art={active.art} radius={5} size={120} label={active.title} /></span>{/if}
        <div class="what">
          <span class="ell">{activity}</span>
          {#if active && !s?.paused && !s?.wait}
            {#if pct != null}
              <span class="bar" role="progressbar" aria-label="Download progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct * 100)}><i style:width="{pct * 100}%"></i></span>
            {:else}
              <span class="bar busy" aria-hidden="true"><i></i></span>
            {/if}
          {/if}
        </div>
        <div class="btns">
          {#if s?.wait === "stopped" || s?.wait === "diskFull"}
            <button class="pill" onclick={() => downloads.retry()}>Try again</button>
          {/if}
          {#if s?.waiting || s?.paused}
            <button class="pill" onclick={() => downloads.pause(!s?.paused)}>
              <Icon name={s?.paused ? "play" : "pause"} size={15} />{s?.paused ? "Resume" : "Pause all"}
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if failed.length}
      <section class="failed" aria-label="Couldn't download">
        <header>
          <h2>Couldn't download</h2>
          <button class="pill" onclick={() => downloads.retry()}>Try all again</button>
        </header>
        <ul>
          {#each failed as f (f.track.id)}
            <li>
              <span class="thumb"><Cover art={f.track.art} radius={4} size={120} label={f.track.title} /></span>
              <span class="txt">
                <span class="t ell">{f.track.title} <small>{artistLine(f.track)}</small></span>
                <span class="why">{f.error ?? "Something went wrong."}</span>
              </span>
              <button class="pill" onclick={() => retryOne(f.track)}>Try again</button>
              <button class="icon-btn" aria-label="Remove {f.track.title} from downloads" title="Remove" onclick={() => downloads.remove([f.track.id])}><Icon name="x" size={16} /></button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <div class="bar-row">
      <div class="chips" role="group" aria-label="Show">
        {#each ["Songs", "Playlists and albums"] as const as t}
          <button class="chip" aria-pressed={tab === t} onclick={() => (tab = t)}>{t}{t === "Playlists and albums" && kept.length ? ` · ${kept.length}` : ""}</button>
        {/each}
      </div>
      {#if tab === "Songs" && songs.length}
        <div class="btns">
          <button class="pill primary" onclick={() => player.playTracks(songs, 0, { label: "Downloads", collectionId: null, radioPlaylist: null })}><Icon name="play" size={15} />Play</button>
          <button class="pill" onclick={shuffle}><Icon name="shuffle" size={15} />Shuffle</button>
        </div>
      {/if}
    </div>

    {#if tab === "Songs"}
      {#if waiting.length}
        <h2 class="sub">Waiting</h2>
        <TrackTable tracks={waiting.map((i) => i.track)} label="Downloads" head={false} />
      {/if}
      {#if songs.length}
        {#if waiting.length}<h2 class="sub">Downloaded</h2>{/if}
        <TrackTable tracks={songs} label="Downloads" />
      {:else if !waiting.length && !failed.length}
        <div class="empty">
          <b>Nothing downloaded yet.</b>
          <p>Choose Download in a song's menu, or download a playlist or album from its page, and it plays here without a connection.</p>
        </div>
      {/if}
    {:else}
      <ul class="kept">
        {#each kept as k (k.card.id)}
          {@const note = keptNote(k, downloads.syncing(k.card.id))}
          <li class:other={!k.mine}>
            <button class="open" onclick={() => nav.go({ name: "collection", id: k.card.id })} oncontextmenu={(e) => keptMenu(e, k)}>
              <span class="art"><Cover art={k.card.art} radius={6} size={120} label={k.card.title} /></span>
              <span class="txt">
                <span class="t ell">{k.card.title}</span>
                <span class="m ell">{kindLabel[k.card.kind]} · {n(k.songs, "song", "songs")} · {sizeText(k.bytes)}</span>
                {#if note}<span class="note ell" class:warn={!!k.error && !k.gone}>{note}</span>{/if}
              </span>
            </button>
            <button class="icon-btn" aria-label="More actions for {k.card.title}" onclick={(e) => keptMenu(e, k)}><Icon name="more" size={18} /></button>
          </li>
        {:else}
          <li class="empty">
            <b>No playlists or albums downloaded.</b>
            <p>Open one and choose Download: its songs download, and new ones follow while Encore is open.</p>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .lede {
    margin-top: 4px;
    color: var(--mute);
    font-size: 13.5px;
  }
  .notice,
  .activity {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 16px;
    padding: 12px 14px 12px 16px;
    border-radius: 12px;
    font-size: 13.5px;
  }
  .notice {
    background: color-mix(in srgb, var(--accent) 12%, var(--panel));
  }
  .notice > div {
    flex: 1;
    min-width: 0;
  }
  .notice span {
    color: var(--mute);
  }
  .notice :global(svg) {
    color: var(--accent);
  }
  .activity {
    background: var(--hover);
  }
  .thumb {
    width: 38px;
    flex: none;
  }
  .what {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 8px;
  }
  .bar {
    position: relative;
    display: block;
    height: 4px;
    border-radius: 2px;
    overflow: hidden;
    background: color-mix(in srgb, var(--text) 14%, transparent);
  }
  .bar i {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 2px;
    background: var(--accent);
    transition: width 0.25s linear;
  }
  .bar.busy i {
    width: 30%;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      left: -30%;
    }
    to {
      left: 100%;
    }
  }
  .btns {
    display: flex;
    gap: 8px;
    flex: none;
  }
  .failed {
    margin-top: 22px;
  }
  .failed header,
  .bar-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .bar-row {
    margin: 26px 0 14px;
  }
  .chips {
    display: flex;
    gap: 8px;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  h2.sub {
    margin: 18px 0 6px 8px;
    font-size: 16px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .failed ul {
    margin-top: 8px;
  }
  .failed li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px;
    border-radius: 8px;
  }
  .failed li:hover {
    background: var(--hover);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: grid;
  }
  .t {
    font-weight: 550;
  }
  .t small {
    margin-left: 6px;
    font-size: 12.5px;
    font-weight: 400;
    color: var(--mute);
  }
  .why,
  .m,
  .note {
    font-size: 12.5px;
    color: var(--mute);
  }
  .note.warn {
    color: hsl(30 80% 62%);
  }
  :global([data-theme="light"]) .note.warn {
    color: hsl(30 80% 36%);
  }
  .kept li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 10px;
  }
  .kept li:hover {
    background: var(--hover);
  }
  .kept li.other .art {
    opacity: 0.7;
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    text-align: left;
  }
  .art {
    width: 56px;
    flex: none;
  }
  .empty {
    display: grid;
    gap: 4px;
    padding: 28px 0;
    color: var(--mute);
    max-width: 56ch;
  }
  .empty b {
    color: var(--text);
    font-size: 16px;
  }
  .kept li.empty {
    display: grid;
  }
  .kept li.empty:hover {
    background: none;
  }
</style>
