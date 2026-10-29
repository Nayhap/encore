<script lang="ts">
  import { ui } from "$lib/state.svelte";
  import { mod, shift } from "$lib/platform";
  import Icon from "./Icon.svelte";

  let menuEl = $state<HTMLDivElement>();
  let pos = $state({ x: 0, y: 0 });

  // Keep the menu on screen and give it keyboard focus.
  $effect(() => {
    if (!ui.menu || !menuEl) return;
    const r = menuEl.getBoundingClientRect();
    pos = {
      x: Math.min(ui.menu.x, innerWidth - r.width - 8),
      y: ui.menu.y + r.height > innerHeight - 8 ? Math.max(8, ui.menu.y - r.height) : ui.menu.y,
    };
    menuEl.querySelector<HTMLElement>("button")?.focus();
  });

  function menuKey(e: KeyboardEvent) {
    const btns = [...menuEl!.querySelectorAll<HTMLElement>("button")];
    const i = btns.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowDown") btns[(i + 1) % btns.length].focus();
    else if (e.key === "ArrowUp") btns[(i - 1 + btns.length) % btns.length].focus();
    else if (e.key === "Escape") ui.menu = null;
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  const shortcuts: [string, string][] = [
    ["Space", "Play or pause"],
    ["← →", `Seek 5 seconds (with ${shift}, 15)`],
    [`${mod("←")} ${mod("→")}`, "Previous or next song"],
    ["[ ]", "Previous or next lyric line"],
    ["S", "Shuffle"],
    ["R", "Change repeat mode"],
    ["L", "Like the current song"],
    ["F", "Full-screen lyrics"],
    ["↑ ↓", "Volume"],
    [mod("K"), "Search and commands"],
    [mod("\\"), "Show or hide now playing panel"],
    [`${mod("[")} ${mod("]")}`, "Back or forward"],
  ];
</script>

{#if ui.menu}
  <div class="catch" role="presentation" onmousedown={() => (ui.menu = null)} oncontextmenu={(e) => { e.preventDefault(); ui.menu = null; }}></div>
  <div class="menu" role="menu" tabindex="-1" bind:this={menuEl} style:left="{pos.x}px" style:top="{pos.y}px" onkeydown={menuKey}>
    {#each ui.menu.items as it}
      <button role="menuitem" onclick={() => { ui.menu = null; it.run(); }}>
        {#if it.icon}<Icon name={it.icon} size={16} />{/if}
        <span>{it.label}</span>
      </button>
    {/each}
  </div>
{/if}

{#if ui.shortcuts}
  <div class="scrim" role="presentation" onmousedown={() => (ui.shortcuts = false)}>
    <div class="sheet" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" tabindex="-1">
      <h2>Keyboard shortcuts</h2>
      <dl>
        {#each shortcuts as [k, v]}
          <dt><kbd>{k}</kbd></dt>
          <dd>{v}</dd>
        {/each}
      </dl>
    </div>
  </div>
{/if}

{#if ui.toast}
  {#key ui.toast.id}
    <div class="toast" role="status">
      <Icon name="check" size={16} />
      {ui.toast.text}
    </div>
  {/key}
{/if}

<style>
  .catch {
    position: fixed;
    inset: 0;
    z-index: 60;
  }
  .menu {
    position: fixed;
    z-index: 61;
    min-width: 220px;
    padding: 5px;
    border-radius: 12px;
    background: var(--raise);
    box-shadow: var(--shadow);
    animation: pop 0.12s var(--ease);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border-radius: 7px;
    font-size: 13.5px;
    color: var(--text);
  }
  .menu button :global(svg) {
    color: var(--mute);
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: var(--accent-soft);
    outline: none;
  }

  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.4);
  }
  .sheet {
    width: min(460px, calc(100vw - 48px));
    padding: 22px 24px;
    border-radius: 16px;
    background: var(--raise);
    box-shadow: var(--shadow);
  }
  .sheet h2 {
    font-family: var(--font-display);
    font-size: 22px;
    margin-bottom: 14px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 9px 16px;
    margin: 0;
    font-size: 13.5px;
  }
  dt {
    text-align: right;
  }
  dd {
    margin: 0;
    color: var(--mute);
  }

  .toast {
    position: fixed;
    left: 50%;
    bottom: calc(var(--deck-h) + 16px);
    transform: translateX(-50%);
    z-index: 70;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 16px 9px 12px;
    border-radius: 99px;
    background: var(--text);
    color: var(--ink);
    font-weight: 600;
    font-size: 13.5px;
    box-shadow: var(--shadow);
    animation: toast 2.4s var(--ease) forwards;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.96);
    }
  }
  @keyframes toast {
    0% {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
    8%,
    85% {
      opacity: 1;
      transform: translate(-50%, 0);
    }
    100% {
      opacity: 0;
    }
  }
</style>
