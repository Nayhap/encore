<script lang="ts">
  import { nav, ui } from "$lib/state.svelte";
  import { mod } from "$lib/platform";
  import Icon from "./Icon.svelte";

  let { title = "", scrolled = false }: { title?: string; scrolled?: boolean } = $props();
</script>

<!-- The whole bar is a window drag handle; buttons inside stay clickable. -->
<header class="top" class:scrolled data-tauri-drag-region>
  <div class="hist">
    <button class="icon-btn" aria-label="Back" title="Back ({mod('[')})" disabled={!nav.canBack} onclick={() => nav.back()}><Icon name="left" size={18} /></button>
    <button class="icon-btn" aria-label="Forward" title="Forward ({mod(']')})" disabled={!nav.canForward} onclick={() => nav.forward()}><Icon name="right" size={18} /></button>
  </div>
  <span class="title" class:show={scrolled} data-tauri-drag-region>{title}</span>
  <div class="right">
    <button class="icon-btn" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)" onclick={() => (ui.shortcuts = true)}><Icon name="keyboard" size={18} /></button>
  </div>
</header>

<style>
  .top {
    position: absolute;
    inset: 0 0 auto 0;
    height: var(--top-h);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px 0 20px;
    z-index: 4;
    transition: background-color 0.2s, box-shadow 0.2s;
  }
  /* With the panel closed, the window buttons sit at this bar's right end. */
  :global(.stage-closed) .top {
    padding-right: calc(16px + var(--wc-w));
  }
  .top.scrolled {
    background: color-mix(in srgb, var(--ink) 82%, transparent);
    backdrop-filter: blur(18px) saturate(1.4);
    -webkit-backdrop-filter: blur(18px) saturate(1.4);
    box-shadow: 0 1px 0 var(--line);
  }
  /* No backdrop blur under WebKitGTK while scrolling; a solid bar instead. */
  :global(:root[data-os="linux"]) .top.scrolled {
    background: color-mix(in srgb, var(--ink) 96%, transparent);
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
  .hist {
    display: flex;
    gap: 2px;
  }
  .title {
    flex: 1;
    font-weight: 650;
    font-size: 14px;
    opacity: 0;
    transform: translateY(4px);
    transition: opacity 0.2s, transform 0.2s var(--ease);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .title.show {
    opacity: 1;
    transform: none;
  }
</style>
