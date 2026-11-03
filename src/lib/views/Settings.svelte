<script lang="ts">
  // Settings, one component per section (./settings), so each can change
  // on its own. The page styles the rows of all of them.
  import Account from "./settings/Account.svelte";
  import Appearance from "./settings/Appearance.svelte";
  import Connections from "./settings/Connections.svelte";
  import Playback from "./settings/Playback.svelte";
  import Storage from "./settings/Storage.svelte";
  import Updates from "./settings/Updates.svelte";
  import { settings } from "./settings/shared.svelte";

  settings.err = null;

  // The section selector: jumps to a section, and follows the one in view.
  const parts = [
    ["account", "Account"],
    ["appearance", "Appearance"],
    ["playback", "Playback"],
    ["connections", "Connections"],
    ["storage", "Storage"],
    ["updates", "Updates"],
  ] as const;
  let current = $state<string>("account");
  let page = $state<HTMLDivElement>();
  let jumping = 0;

  function go(id: string) {
    current = id;
    jumping = Date.now();
    page?.querySelector(`#set-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  $effect(() => {
    const scroller = page?.closest<HTMLElement>(".scroll");
    if (!scroller || !page) return;
    const els = parts.map(([id]) => page!.querySelector<HTMLElement>(`#set-${id}`)!);
    const follow = () => {
      if (Date.now() - jumping < 700) return;
      const top = scroller.getBoundingClientRect().top + 140;
      // At the bottom the last section counts, however short it is.
      const end = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 4;
      let i = end ? els.length - 1 : 0;
      if (!end) els.forEach((el, j) => el && el.getBoundingClientRect().top <= top && (i = j));
      current = parts[i][0];
    };
    scroller.addEventListener("scroll", follow, { passive: true });
    return () => scroller.removeEventListener("scroll", follow);
  });
</script>

<div class="page settings" bind:this={page}>
  <h1 class="page-title">Settings</h1>
  <nav class="parts" aria-label="Settings sections">
    {#each parts as [id, name] (id)}
      <button aria-current={current === id ? "true" : undefined} onclick={() => go(id)}>{name}</button>
    {/each}
  </nav>
  <div id="set-account"><Account /></div>
  <div id="set-appearance"><Appearance /></div>
  <div id="set-playback"><Playback /></div>
  <div id="set-connections"><Connections /></div>
  <div id="set-storage"><Storage /></div>
  <div id="set-updates"><Updates /></div>
</div>

<style>
  .parts {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    gap: 2px;
    max-width: 720px;
    margin: 14px 0 -8px;
    padding: 8px 0;
    background: var(--ink);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .parts button {
    flex: none;
    height: 30px;
    padding: 0 12px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 550;
    color: var(--mute);
    transition: background-color 0.15s, color 0.15s;
  }
  .parts button:hover {
    color: var(--text);
    background: var(--hover);
  }
  .parts button[aria-current="true"] {
    color: var(--text);
    background: var(--hover);
  }
  [id^="set-"] {
    scroll-margin-top: 104px;
  }
  .settings :global(section) {
    max-width: 720px;
    margin-top: 28px;
  }
  .settings :global(h2) {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--dim);
    margin-bottom: 6px;
  }
  .settings :global(.row) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .settings :global(.row p) {
    margin-top: 2px;
    font-size: 13px;
    color: var(--mute);
    max-width: 52ch;
  }
  .settings :global(.seg) {
    display: flex;
    padding: 3px;
    gap: 2px;
    border-radius: 10px;
    background: var(--hover);
  }
  .settings :global(.seg button) {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 550;
    color: var(--mute);
  }
  .settings :global(.seg button[aria-checked="true"]) {
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.2);
  }
  .settings :global(.btns) {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .settings :global(select) {
    height: 34px;
    max-width: 240px;
    padding: 0 10px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: var(--hover);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .settings :global(.grow) {
    display: grid;
    justify-items: start;
  }
  .settings :global(.link) {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 6px;
    font-size: 13px;
    font-weight: 550;
    color: var(--text);
  }
  .settings :global(.link:hover) {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .settings :global(.note) {
    margin-top: 10px;
    font-size: 13px;
    color: var(--mute);
  }
  .settings :global(.note.error) {
    color: hsl(0 70% 62%);
  }
  .settings :global(.picker) {
    display: grid;
    gap: 8px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .settings :global(.chan) {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--hover);
  }
  .settings :global(.chan small) {
    color: var(--mute);
  }
  .settings :global(.picker .btns) {
    justify-content: flex-start;
  }
</style>
