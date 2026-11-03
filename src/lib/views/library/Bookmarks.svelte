<script lang="ts">
  // Library → Bookmarks (D25): moments, songs and pages, newest first.
  // A moment plays from its time; a song plays; a page opens.
  import type { Bookmark } from "$lib/backend";
  import { fmt } from "$lib/data";
  import { bookmarkSub, downloads, openBookmark, player, removeBookmark, saved, ui, type MenuItem } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";

  const groups = $derived(
    (
      [
        ["Moments", saved.bookmarks.filter((b) => b.kind === "moment")],
        ["Songs", saved.bookmarks.filter((b) => b.kind === "track")],
        ["Pages", saved.bookmarks.filter((b) => b.kind === "page")],
      ] as [string, Bookmark[]][]
    ).filter(([, l]) => l.length),
  );

  const round = (b: Bookmark) => b.target.type === "page" && b.target.value.name === "artist";
  const length = (b: Bookmark) => (b.target.type === "track" ? b.target.value.duration : null);
  const playable = (b: Bookmark) => b.target.type === "page" || downloads.playable(b.target.value);
  const isNow = (b: Bookmark) => b.target.type === "track" && player.current?.id === b.target.value.id;

  function menu(e: MouseEvent, b: Bookmark) {
    const items: MenuItem[] = [
      { label: b.target.type === "page" ? "Open" : b.kind === "moment" ? `Play from ${fmt(b.position)}` : "Play", icon: b.target.type === "page" ? "right" : "play", run: () => openBookmark(b) },
      { label: "Remove bookmark", icon: "x", run: () => void removeBookmark(b) },
    ];
    ui.openMenu(e, items);
  }
</script>

{#if !saved.bookmarks.length}
  <div class="empty">
    <div class="glyph" aria-hidden="true"><Icon name="bookmark" size={30} /></div>
    <h2>Nothing bookmarked yet</h2>
    <p>Press <kbd>B</kbd> while a song plays to keep that moment. To keep a song, a playlist, an album or an artist, choose Bookmark in its menu.</p>
  </div>
{:else}
  {#each groups as [title, list] (title)}
    <section>
      <h2 class="group">{title}</h2>
      <ul>
        {#each list as b (b.id)}
          {@const d = length(b)}
          <li class:dim={!playable(b)} class:now={isNow(b)}>
            <button class="open" onclick={() => openBookmark(b)} oncontextmenu={(e) => menu(e, b)} aria-label="{b.kind === 'page' ? 'Open' : 'Play'} {b.title}{b.kind === 'moment' ? ` from ${fmt(b.position)}` : ''}">
              <span class="thumb" class:round={round(b)}><Cover art={b.art} round={round(b)} radius={6} size={120} label={b.title} /></span>
              <span class="txt">
                <span class="t ell">{b.title}</span>
                <span class="s ell">{b.kind === "moment" ? b.subtitle : bookmarkSub(b)}</span>
              </span>
              {#if b.kind === "moment" && b.position != null}
                <span class="at" title="Plays from {fmt(b.position)}">
                  <span class="time mono"><Icon name="play" size={10} />{fmt(b.position)}</span>
                  {#if d}<span class="track" aria-hidden="true"><i style:left="{Math.min(100, (b.position / d) * 100)}%"></i></span>{/if}
                </span>
              {/if}
            </button>
            <button class="icon-btn rm" aria-label="Remove bookmark of {b.title}" title="Remove bookmark" onclick={() => removeBookmark(b)}><Icon name="x" size={15} /></button>
          </li>
        {/each}
      </ul>
    </section>
  {/each}
{/if}

<style>
  section + section {
    margin-top: 26px;
  }
  .group {
    margin: 0 0 6px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
  }
  ul {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
  }
  li {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: 9px;
  }
  li:hover {
    background: var(--hover);
  }
  li.dim .open {
    opacity: 0.45;
  }
  .open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 7px 8px;
    text-align: left;
  }
  .thumb {
    width: 48px;
    flex: none;
  }
  .txt {
    flex: 1;
    display: grid;
    min-width: 0;
  }
  .t {
    font-weight: 600;
  }
  li.now .t {
    color: var(--accent);
  }
  .s {
    font-size: 13px;
    color: var(--mute);
  }
  .at {
    display: grid;
    justify-items: end;
    gap: 5px;
    flex: none;
    width: 92px;
  }
  .time {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px 0 8px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-soft);
  }
  /* Where the moment falls in the song. */
  .track {
    position: relative;
    width: 100%;
    height: 3px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--text) 12%, transparent);
  }
  .track i {
    position: absolute;
    top: 50%;
    width: 7px;
    height: 7px;
    margin: -3.5px 0 0 -3.5px;
    border-radius: 50%;
    background: var(--accent);
  }
  .rm {
    width: 30px;
    height: 30px;
    margin-right: 6px;
    opacity: 0;
  }
  li:hover .rm,
  .rm:focus-visible {
    opacity: 1;
  }
  .empty {
    display: grid;
    justify-items: center;
    gap: 8px;
    max-width: 420px;
    margin: 40px auto 0;
    text-align: center;
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 64px;
    height: 64px;
    margin-bottom: 6px;
    border-radius: 18px;
    color: var(--accent);
    background: var(--accent-soft);
  }
  .empty h2 {
    font-family: var(--font-display);
    font-size: 19px;
    font-weight: 650;
  }
  .empty p {
    color: var(--mute);
    font-size: 13.5px;
    line-height: 1.5;
    text-wrap: pretty;
  }
</style>
