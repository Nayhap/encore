<script lang="ts">
  // Select mode's bar (D34): it takes the toolbar's place and stays at the
  // top while the list scrolls. Every action works on the selected rows;
  // writes say why they can't run (offline, signed out) instead of hiding.
  import { account, downloads, likeMany, needsOnline, pickPlaylists, player, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import Glyph from "./Glyph.svelte";
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
  const write = $derived(!account.signedIn ? "Sign in to change your library." : needsOnline("change your library"));

  /** Run an action on the selection, or say why it can't run. */
  function act(f: () => void, blocked?: string | null) {
    if (blocked) return ui.say(blocked);
    if (!n) return ui.say("Select some songs first");
    f();
  }
  function queueMenu(e: MouseEvent) {
    if (!n) return ui.say("Select some songs first");
    ui.openMenu(e, [
      { label: "Play next", icon: "playNext", run: () => (player.playNext(ids), ui.say(`${songs(n)} play next`)) },
      { label: "Add to queue", icon: "queue", run: () => (player.addToQueue(ids), ui.say(`Added ${songs(n)} to the queue`)) },
    ]);
  }
  const playable = $derived(tools.chosen.filter((e) => e.track.playable).map((e) => e.track.id));
</script>

<div class="bar" role="toolbar" aria-label="Selected songs">
  <button class="icon-btn done" aria-label="Done selecting" title="Done (Esc)" onclick={() => tools.done()}><Icon name="x" size={18} /></button>
  <div class="count">
    <b>{n ? `${n.toLocaleString()} selected` : "Select songs"}</b>
    <button class="all" onclick={() => tools.toggleAll()}>{tools.allShown ? "Select none" : tools.filtered ? "Select all shown" : "Select all"}</button>
  </div>
  <div class="acts">
    <button class="act" class:off={!!write} title={write ?? "Add to playlists"} onclick={() => act(() => pickPlaylists(ids, from), write)}><Icon name="plus" size={17} /><span>Add to playlists</span></button>
    {#if onremove || removeNote}
      <button class="act" class:off={!!write || !onremove} title={write ?? removeNote ?? "Remove from this playlist"} onclick={() => act(() => onremove?.(), write ?? (onremove ? null : removeNote))}><Icon name="trash" size={17} /><span>Remove</span></button>
    {/if}
    <button class="act" class:off={!!write} title={write ?? "Add to liked music"} onclick={() => act(() => void likeMany(ids, true), write)}><Icon name="heart" size={17} /><span>Like</span></button>
    <button class="act" class:off={!!write} title={write ?? "Remove from liked music"} onclick={() => act(() => void likeMany(ids, false), write)}><Glyph name="unlike" size={17} /><span>Unlike</span></button>
    {#if downloads.available}
      <button class="act" title="Download for offline listening" onclick={() => act(() => (playable.length ? void downloads.download(playable) : ui.say("These songs can't be downloaded")))}><Icon name="download" size={17} /><span>Download</span></button>
    {/if}
    <button class="act" title="Play next or add to the queue" onclick={queueMenu}><Icon name="queue" size={17} /><span>Queue</span></button>
  </div>
</div>
{#if tools.reviewing}
  <p class="review">
    {#if tools.dups.copies.size}<span><i class="dot copy"></i>{tools.dups.copies.size === 1 ? "1 copy" : `${tools.dups.copies.size} copies`} of songs listed earlier, selected</span>{/if}
    {#if tools.dups.versions.size}<span><i class="dot version"></i>{tools.dups.versions.size === 1 ? "1 song" : `${tools.dups.versions.size} songs`} also here as the other version (song or video): yours to check</span>{/if}
  </p>
{/if}

<svelte:window onkeydown={(e) => e.key === "Escape" && !ui.menu && !ui.confirm && !ui.sheet && tools.done()} />

<style>
  .bar {
    position: sticky;
    top: 8px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 52px;
    margin: 0 -8px 10px;
    padding: 6px 8px 6px 6px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--accent) 12%, var(--raise));
    box-shadow: var(--shadow);
    animation: in 0.2s var(--ease);
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
  .all {
    justify-self: start;
    font-size: 12.5px;
    color: var(--accent);
    font-weight: 600;
  }
  .all:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
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
    transition: background-color 0.15s;
  }
  .act:hover {
    background: color-mix(in srgb, var(--text) 9%, transparent);
  }
  .act.off {
    opacity: 0.4;
  }
  .review {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 18px;
    margin: -2px 0 12px;
    font-size: 13px;
    color: var(--mute);
  }
  .review span {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot.copy {
    background: var(--accent);
  }
  .dot.version {
    box-shadow: inset 0 0 0 1.5px var(--mute);
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @media (max-width: 1180px) {
    .act span {
      display: none;
    }
    .act {
      width: 38px;
      padding: 0;
      justify-content: center;
    }
  }
</style>
