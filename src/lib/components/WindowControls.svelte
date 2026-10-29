<script lang="ts">
  // Minimise, maximise/restore and close for Windows and Linux, where the
  // window has no native title bar. Drawn in Encore's own style with theme
  // colours, so they read in light and dark. On Windows, resting on maximise
  // opens snap layouts as the native button would.
  import { onMount } from "svelte";
  import { isTauri } from "$lib/backend";
  import { commands } from "$lib/backend/bindings";
  import { os } from "$lib/platform";

  let maximized = $state(false);
  let focused = $state(true);

  const win = async () => (await import("@tauri-apps/api/window")).getCurrentWindow();

  onMount(() => {
    if (!isTauri) return;
    let off: (() => void)[] = [];
    (async () => {
      const w = await win();
      maximized = await w.isMaximized();
      off.push(await w.onResized(async () => (maximized = await w.isMaximized())));
      off.push(await w.onFocusChanged(({ payload }) => (focused = payload)));
    })();
    return () => off.forEach((f) => f());
  });

  const minimize = async () => isTauri && (await win()).minimize();
  const toggle = async () => isTauri && (await win()).toggleMaximize();
  const close = async () => isTauri && (await win()).close();

  let snapTimer: ReturnType<typeof setTimeout> | undefined;
  function snapEnter() {
    if (os !== "windows" || !isTauri) return;
    clearTimeout(snapTimer);
    snapTimer = setTimeout(() => {
      if (document.hasFocus()) commands.snapLayouts().catch(() => {});
    }, 650);
  }
  const snapLeave = () => clearTimeout(snapTimer);
</script>

<div class="wc" class:blur={!focused} role="group" aria-label="Window">
  <button aria-label="Minimise" title="Minimise" onclick={minimize}>
    <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2 6.5h8" /></svg>
  </button>
  <button aria-label={maximized ? "Restore" : "Maximise"} title={maximized ? "Restore" : "Maximise"} onclick={toggle} onmouseenter={snapEnter} onmouseleave={snapLeave} onmousedown={snapLeave}>
    {#if maximized}
      <svg viewBox="0 0 12 12" aria-hidden="true"><rect x="2" y="4" width="6" height="6" rx="1.2" /><path d="M4.2 2h4.6A1.2 1.2 0 0 1 10 3.2v4.6" /></svg>
    {:else}
      <svg viewBox="0 0 12 12" aria-hidden="true"><rect x="2.25" y="2.25" width="7.5" height="7.5" rx="1.4" /></svg>
    {/if}
  </button>
  <button class="close" aria-label="Close" title="Close" onclick={close}>
    <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M2.75 2.75l6.5 6.5M9.25 2.75l-6.5 6.5" /></svg>
  </button>
</div>

<style>
  .wc {
    position: fixed;
    top: 10px;
    right: 10px;
    z-index: 80;
    display: flex;
    gap: 4px;
  }
  button {
    display: grid;
    place-items: center;
    width: 40px;
    height: 32px;
    border-radius: 9px;
    color: var(--mute);
    transition: color 0.15s, background-color 0.15s, opacity 0.15s;
  }
  .wc.blur button {
    opacity: 0.6;
  }
  button:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text) 10%, transparent);
    opacity: 1;
  }
  button:active {
    background: color-mix(in srgb, var(--text) 16%, transparent);
  }
  .close:hover,
  .close:active {
    color: #fff;
    background: #d92c3c;
  }
  svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.25;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
