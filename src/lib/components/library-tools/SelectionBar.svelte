<script lang="ts">
  // Select mode's bar (D34): it takes the toolbar's place and stays at the
  // top while the list scrolls. Every action works on the selected rows and
  // waits until there are some; writes say why they can't run (offline,
  // signed out) instead of hiding. Reviewing duplicates it is a cleanup bar
  // of its own: the extra copies, and removing them.
  import { account, downloads, grouped, likeMany, needsOnline, pickPlaylists, player, ui, type MenuItem } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import type { ListTools } from "./tools.svelte";

  let {
    tools,
    from = null,
    onremove,
    removeNote,
  }: {
    tools: ListTools;
    /** The playlist these rows are in, left out of "Add to playlists". */
    from?: string | null;
    /** Remove the selected rows from this playlist (yours only). */
    onremove?: () => void;
    /** Why rows can't be removed, when they can't. */
    removeNote?: string;
  } = $props();

  const n = $derived(tools.chosen.length);
  const ids = $derived(tools.chosen.map((e) => e.track.id));
  const songs = (k: number) => (k === 1 ? "1 song" : `${k.toLocaleString()} songs`);
  const copies = (k: number) => (k === 1 ? "1 extra copy" : `${k.toLocaleString()} extra copies`);
  const write = $derived(!account.signedIn ? "Sign in to change your library." : needsOnline("change your library"));

  /** Run an action on the selection, or say why it can't run. */
  function act(f: () => void, blocked?: string | null) {
    if (blocked) return ui.say(blocked);
    f();
  }
  const playable = $derived(tools.chosen.filter((e) => e.track.playable).map((e) => e.track.id));

  function more(e: MouseEvent) {
    const liked = ids.filter((id) => player.liked.has(id)).length;
    const likes: MenuItem[] = [];
    if (liked < n) likes.push({ label: "Add to liked music", icon: "heart", disabled: write ?? undefined, run: () => void likeMany(ids, true) });
    if (liked > 0) likes.push({ label: "Remove from liked music", icon: "heartFill", disabled: write ?? undefined, run: () => void likeMany(ids, false) });
    const dl: MenuItem[] = downloads.available
      ? [{ label: "Download", icon: "download", run: () => (playable.length ? void downloads.download(playable) : ui.say("These songs can't be downloaded")) }]
      : [];
    ui.openMenu(
      e,
      grouped(
        [
          { label: "Play next", icon: "playNext", run: () => (player.playNext(ids), ui.say(`${songs(n)} play next`)) },
          { label: "Add to queue", icon: "queue", run: () => (player.addToQueue(ids), ui.say(`Added ${songs(n)} to the queue`)) },
        ],
        likes,
        dl,
      ),
    );
  }

  // Reviewing: what the selection is besides extra copies.
  const extra = $derived(tools.beyondCopies);
  const onlyCopies = $derived(extra.firsts + extra.versions === 0);
</script>

<div class="stick">
  {#if tools.reviewing}
    <div class="bar review" role="toolbar" aria-label="Duplicates">
      <div class="count">
        <b>{n === 0 ? "Nothing selected" : onlyCopies ? `${copies(n)} selected` : `${songs(n)} selected`}</b>
        {#if !tools.onlyCopies && tools.dups.copies.size}
          <button class="link" onclick={() => tools.selectCopies()}>Select extra copies</button>
        {:else if tools.dups.versions.size}
          <span class="hint">Song and video pairs are yours to check</span>
        {/if}
      </div>
      <div class="acts">
        {#if onremove}
          <button class="act primary" disabled={!n} class:off={!!write} title={write ?? undefined} onclick={() => act(() => onremove?.(), write)}>
            <Icon name="trash" size={16} /><span>{onlyCopies ? "Remove selected copies" : "Remove selected"}</span>
          </button>
        {/if}
        <button class="act" onclick={() => tools.done()}>Done</button>
      </div>
    </div>
  {:else}
    <div class="bar" role="toolbar" aria-label="Selected songs">
      <button class="icon-btn done" aria-label="Done selecting" title="Done (Esc)" onclick={() => tools.done()}><Icon name="x" size={18} /></button>
      <div class="count">
        <b>{n ? `${n.toLocaleString()} selected` : "Select songs"}</b>
        <button class="link" onclick={() => tools.toggleAll()}>{tools.allShown ? "Select none" : tools.filtered ? "Select all shown" : "Select all"}</button>
      </div>
      <div class="acts">
        <button class="act" disabled={!n} class:off={!!write} title={write ?? "Add to playlists"} onclick={() => act(() => pickPlaylists(ids, from), write)}><Icon name="plus" size={17} /><span>Add to playlists</span></button>
        {#if onremove || removeNote}
          <button class="act" disabled={!n} class:off={!!write || !onremove} title={write ?? removeNote ?? "Remove from this playlist"} onclick={() => act(() => onremove?.(), write ?? (onremove ? null : removeNote))}><Icon name="trash" size={17} /><span>Remove</span></button>
        {/if}
        <button class="act" disabled={!n} aria-haspopup="menu" title="Like, download or queue" onclick={more}><Icon name="more" size={17} /><span>More</span></button>
      </div>
    </div>
  {/if}
</div>

<svelte:window onkeydown={(e) => e.key === "Escape" && !ui.menu && !ui.confirm && !ui.sheet && tools.done()} />

<style>
  .stick {
    position: sticky;
    top: 8px;
    z-index: 5;
    margin: 0 -8px 10px;
    container-type: inline-size;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 52px;
    padding: 6px 8px 6px 6px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--accent) 12%, var(--raise));
    box-shadow: var(--shadow);
    animation: in 0.2s var(--ease);
  }
  .bar.review {
    padding-left: 16px;
  }
  .count {
    display: grid;
    min-width: 0;
    line-height: 1.25;
  }
  .count b {
    font-size: 14px;
    font-weight: 650;
    white-space: nowrap;
  }
  .link {
    justify-self: start;
    font-size: 12.5px;
    color: var(--accent);
    font-weight: 600;
  }
  .link:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .hint {
    font-size: 12.5px;
    color: var(--mute);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .acts {
    display: flex;
    gap: 2px;
    margin-left: auto;
  }
  .act {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 36px;
    padding: 0 12px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    transition: background-color 0.15s, opacity 0.15s;
  }
  .act:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 9%, transparent);
  }
  .act.primary {
    padding: 0 14px;
    color: var(--accent-ink);
    background: var(--accent);
  }
  .act.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 88%, var(--text));
  }
  .act:disabled,
  .act.off {
    opacity: 0.4;
    cursor: default;
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  /* A narrow list keeps the words and drops the icons. */
  @container (max-width: 560px) {
    .act :global(svg) {
      display: none;
    }
    .act {
      padding: 0 10px;
    }
    .hint {
      display: none;
    }
  }
</style>
