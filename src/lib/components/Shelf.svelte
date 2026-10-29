<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let { title, sub, onall, children }: { title: string; sub?: string; onall?: () => void; children: Snippet } = $props();

  let row: HTMLDivElement;
  let atStart = $state(true);
  let atEnd = $state(false);

  function update() {
    atStart = row.scrollLeft < 4;
    atEnd = row.scrollLeft + row.clientWidth >= row.scrollWidth - 4;
  }
  $effect(() => {
    update();
    const ro = new ResizeObserver(update);
    ro.observe(row);
    return () => ro.disconnect();
  });
  const page = (dir: number) => row.scrollBy({ left: dir * row.clientWidth * 0.85, behavior: "smooth" });
</script>

<section class="shelf">
  <header>
    <div>
      {#if sub}<p class="sub">{sub}</p>{/if}
      <h2>{title}</h2>
    </div>
    <div class="ctl">
      {#if onall}<button class="all" onclick={onall}>Show all</button>{/if}
      <button class="icon-btn" aria-label="Scroll left" disabled={atStart} onclick={() => page(-1)}><Icon name="left" size={17} /></button>
      <button class="icon-btn" aria-label="Scroll right" disabled={atEnd} onclick={() => page(1)}><Icon name="right" size={17} /></button>
    </div>
  </header>
  <div class="row" class:fade-l={!atStart} class:fade-r={!atEnd} bind:this={row} onscroll={update}>
    {@render children()}
  </div>
</section>

<style>
  .shelf {
    margin-top: 34px;
  }
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  .sub {
    font-size: 12.5px;
    color: var(--mute);
    margin-bottom: 2px;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 23px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }
  .ctl {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .all {
    font-size: 13px;
    font-weight: 600;
    color: var(--mute);
    padding: 6px 10px;
    border-radius: 99px;
    margin-right: 4px;
  }
  .all:hover {
    color: var(--text);
    background: var(--hover);
  }
  .row {
    display: flex;
    gap: 20px;
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    scroll-padding: 0 32px;
    scrollbar-width: none;
    margin: 0 -32px;
    padding: 4px 32px 6px;
  }
  .row::-webkit-scrollbar {
    display: none;
  }
  .row.fade-r {
    mask-image: linear-gradient(90deg, #000 calc(100% - 64px), transparent);
  }
  .row.fade-l {
    mask-image: linear-gradient(90deg, transparent, #000 64px);
  }
  .row.fade-l.fade-r {
    mask-image: linear-gradient(90deg, transparent, #000 64px, #000 calc(100% - 64px), transparent);
  }
</style>
