<script lang="ts">
  import type { ShelfItem } from "$lib/backend";
  import { artistLine, kindLabel } from "$lib/data";
  import { collectionMenu, nav, playArtist, player, trackMenu, ui } from "$lib/state.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let { item, size = 176 }: { item: ShelfItem; size?: number } = $props();

  const info = $derived.by(() => {
    if (item.type === "artist") return { art: item.art, title: item.name, sub: item.subtitle || "Artist", round: true };
    if (item.type === "track") return { art: item.art, title: item.title, sub: `${item.kind === "video" ? "Video" : "Song"} · ${artistLine(item)}`, round: false };
    if (item.type === "collection") return { art: item.art, title: item.title, sub: item.subtitle || kindLabel[item.kind], round: false };
    return { art: null, title: item.title, sub: "", round: false };
  });

  const isCurrent = $derived(
    (item.type === "collection" && player.source?.collectionId === item.id) ||
      (item.type === "track" && player.current?.id === item.id) ||
      (item.type === "artist" && player.source?.label === item.name),
  );

  function open() {
    if (item.type === "artist") nav.go({ name: "artist", id: item.id });
    else if (item.type === "collection") nav.go({ name: "collection", id: item.id });
    else if (item.type === "track") player.playTrack(item.id);
    else nav.go({ name: "page", browseId: item.browseId, params: item.params, title: item.title });
  }

  function play() {
    if (isCurrent) return player.toggle();
    if (item.type === "collection") player.playCollection(item.id);
    else if (item.type === "track") player.playTrack(item.id);
    else if (item.type === "artist") playArtist(item.id);
    else open();
  }

  function menu(e: MouseEvent) {
    if (item.type === "track") ui.openMenu(e, trackMenu(item.id));
    else if (item.type === "collection") ui.openMenu(e, collectionMenu(item.id));
  }
</script>

<div class="tile" style:width="{size}px" oncontextmenu={menu} role="group" aria-label={info.title}>
  <div class="frame">
    <button class="art" class:round={info.round} onclick={open} aria-label="Open {info.title}">
      <Cover art={info.art} round={info.round} radius={10} size={size > 200 ? 544 : 226} label={info.title} />
    </button>
    <button class="play" class:on={isCurrent && player.playing} onclick={play} aria-label={isCurrent && player.playing ? "Pause" : `Play ${info.title}`}>
      <Icon name={isCurrent && player.playing ? "pause" : "play"} size={18} />
    </button>
  </div>
  <button class="txt" class:center={info.round} onclick={open} tabindex="-1">
    <span class="t clamp2" title={info.title}>{info.title}</span>
    <span class="s ell">{info.sub}</span>
  </button>
</div>

<style>
  .tile {
    flex: none;
    scroll-snap-align: start;
  }
  .frame {
    position: relative;
  }
  .art {
    display: block;
    width: 100%;
    border-radius: 10px;
    transition: transform 0.25s var(--ease), box-shadow 0.25s;
  }
  .art.round {
    border-radius: 50%;
  }
  .tile:hover .art {
    transform: translateY(-2px);
    box-shadow: 0 14px 30px -14px rgb(0 0 0 / 0.6);
  }
  .play {
    position: absolute;
    right: 10px;
    bottom: 10px;
    display: grid;
    place-items: center;
    width: 42px;
    height: 42px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-ink);
    box-shadow: 0 8px 20px -6px rgb(0 0 0 / 0.5);
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 0.18s, transform 0.25s var(--ease);
  }
  .art.round + .play {
    right: 4px;
    bottom: 4px;
  }
  .tile:hover .play,
  .play:focus-visible,
  .play.on {
    opacity: 1;
    transform: none;
  }
  .txt {
    display: grid;
    gap: 2px;
    width: 100%;
    margin-top: 10px;
    text-align: left;
  }
  .txt.center {
    text-align: center;
  }
  .t {
    font-weight: 600;
    font-size: 14px;
    line-height: 1.3;
  }
  .s {
    font-size: 12.5px;
    color: var(--mute);
  }
</style>
