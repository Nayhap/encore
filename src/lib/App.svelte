<script lang="ts">
  import "@fontsource-variable/bricolage-grotesque/standard.css";
  import "@fontsource-variable/instrument-sans/standard.css";
  import "@fontsource-variable/noto-sans-jp";
  import "@fontsource/ibm-plex-mono/400.css";
  import "@fontsource/ibm-plex-mono/500.css";
  import "./styles/global.css";

  import { onMount } from "svelte";
  import { isTauri } from "./backend";
  import { commands } from "./backend/bindings";
  import { isMac } from "./platform";
  import { catalog } from "./catalog.svelte";
  import { account, connect, nav, player, ui } from "./state.svelte";
  import Deck from "./components/Deck.svelte";
  import Overlays from "./components/Overlays.svelte";
  import Palette from "./components/Palette.svelte";
  import PlaylistDialog from "./components/PlaylistDialog.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import Stage from "./components/Stage.svelte";
  import Theater from "./components/Theater.svelte";
  import TopBar from "./components/TopBar.svelte";
  import WindowControls from "./components/WindowControls.svelte";
  import Artist from "./views/Artist.svelte";
  import Collection from "./views/Collection.svelte";
  import Explore from "./views/Explore.svelte";
  import History from "./views/History.svelte";
  import Home from "./views/Home.svelte";
  import Library from "./views/Library.svelte";
  import BrowsePage from "./views/BrowsePage.svelte";
  import Settings from "./views/Settings.svelte";

  let scroller: HTMLDivElement;
  let scrolled = $state(false);

  // Narrow windows start with the panel closed so browsing keeps its room.
  if (innerWidth < 1180) ui.stageOpen = false;

  let connectError = $state<string | null>(null);
  onMount(() => {
    connect().catch((e) => (connectError = e?.message ?? String(e)));
  });

  const viewTitle = $derived.by(() => {
    const v = nav.view;
    if (v.name === "collection") return catalog.collection(v.id)?.title ?? "";
    if (v.name === "artist") {
      const a = catalog.artist(v.id);
      return a ? a.name : "";
    }
    if (v.name === "page") return v.title ?? "";
    return v.name[0].toUpperCase() + v.name.slice(1);
  });

  // Each page starts at the top.
  $effect(() => {
    nav.view;
    scroller?.scrollTo({ top: 0 });
    scrolled = false;
  });

  // Theme: resolve "system" live, and never leave window controls unreadable.
  $effect(() => {
    localStorage.setItem("theme", ui.theme);
    const mq = matchMedia("(prefers-color-scheme: light)");
    const apply = () => {
      document.documentElement.dataset.theme = ui.theme === "system" ? (mq.matches ? "light" : "dark") : ui.theme;
    };
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  });

  // Windows and Linux: Encore's own window buttons, or the OS title bar.
  const customChrome = $derived(!isMac && !ui.systemTitlebar);
  $effect(() => {
    document.documentElement.dataset.chrome = customChrome ? "custom" : "native";
  });
  // The backend applies the choice when the window is created; keep its copy
  // in step with this one (earlier versions only kept it here).
  if (isTauri && !isMac) commands.setSystemTitlebar(ui.systemTitlebar, false).catch(() => {});

  $effect(() => {
    localStorage.setItem("tint", ui.tint ? "on" : "off");
    document.documentElement.dataset.tint = ui.tint ? "on" : "off";
  });

  $effect(() => {
    const tr = player.current;
    if (!tr) return;
    const [h1, h2] = catalog.hue(tr.art, tr.id);
    const root = document.documentElement.style;
    root.setProperty("--h", String(h1));
    root.setProperty("--h2", String(h2));
  });

  function lyricJump(dir: 1 | -1) {
    const ls = player.timedLines;
    if (!ls?.length) return;
    const p = player.position;
    if (dir === 1) {
      const n = ls.find((l) => (l.t ?? 0) > p + 0.2);
      if (n) player.seek(n.t ?? 0);
    } else {
      // Jump to the start of the current line, or the previous one if already there.
      const before = ls.filter((l) => (l.t ?? 0) < p - 1.2);
      player.seek(before.at(-1)?.t ?? 0);
    }
  }

  function onkey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    const typing = t.matches("input:not([type=range]):not([type=checkbox]), textarea, select, [contenteditable]");
    const mod = e.metaKey || e.ctrlKey;

    if (mod && e.key.toLowerCase() === "k") {
      e.preventDefault();
      ui.palette = !ui.palette;
      return;
    }
    if (e.key === "Escape") {
      if (ui.menu) ui.menu = null;
      else if (ui.confirm) ui.confirm = null;
      else if (ui.sheet) ui.sheet = null;
      else if (ui.shortcuts) ui.shortcuts = false;
      else if (ui.palette) ui.palette = false;
      else if (ui.theater) ui.theater = false;
      return;
    }
    if (typing || ui.palette || ui.menu || ui.sheet || ui.confirm) return;

    if (mod) {
      if (e.key === "ArrowLeft") player.prev();
      else if (e.key === "ArrowRight") player.next();
      else if (e.key === "ArrowUp") player.setVolume(player.volume + 0.05);
      else if (e.key === "ArrowDown") player.setVolume(player.volume - 0.05);
      else if (e.key === "\\") ui.stageOpen = !ui.stageOpen;
      else if (e.key === "[") nav.back();
      else if (e.key === "]") nav.forward();
      else return;
      e.preventDefault();
      return;
    }
    if (e.altKey) return;

    switch (e.key) {
      case " ":
        player.toggle();
        break;
      case "ArrowLeft":
        if (t.matches("input[type=range]")) return;
        player.seek(player.position - (e.shiftKey ? 15 : 5));
        break;
      case "ArrowRight":
        if (t.matches("input[type=range]")) return;
        player.seek(player.position + (e.shiftKey ? 15 : 5));
        break;
      case "[":
        lyricJump(-1);
        break;
      case "]":
        lyricJump(1);
        break;
      case "s":
        player.toggleShuffle();
        ui.say(player.shuffle ? "Shuffle on" : "Shuffle off");
        break;
      case "r":
        player.cycleRepeat();
        ui.say(player.repeat === "off" ? "Repeat off" : player.repeat === "all" ? "Repeating the queue" : "Repeating this song");
        break;
      case "l":
        if (player.current) {
          player.toggleLike(player.current.id);
          ui.say(player.liked.has(player.current.id) ? "Added to liked music" : "Removed from liked music");
        }
        break;
      case "f":
        ui.theater = !ui.theater;
        break;
      case "/":
        ui.palette = true;
        break;
      case "?":
        ui.shortcuts = true;
        break;
      default:
        return;
    }
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="app" class:stage-closed={!ui.stageOpen}>
  <Sidebar />
  <main>
    <TopBar title={viewTitle} {scrolled} />
    <div class="scroll" bind:this={scroller} onscroll={() => (scrolled = scroller.scrollTop > 24)}>
      {#key JSON.stringify(nav.view)}
        <div class="view">
          {#if account.state.state === "expired"}
            <div class="banner" role="alert">
              <span><b>Your YouTube Music session expired.</b> {account.state.reason}</span>
              <button class="pill primary" onclick={() => nav.go({ name: "settings" })}>Import again</button>
            </div>
          {/if}
          {#if connectError}
            <div class="banner" role="alert">
              <span><b>Encore could not start its backend.</b> {connectError}</span>
              <button class="pill" onclick={() => location.reload()}>Retry</button>
            </div>
          {/if}
          {#if nav.view.name === "home"}
            <Home />
          {:else if nav.view.name === "explore"}
            <Explore />
          {:else if nav.view.name === "library"}
            <Library />
          {:else if nav.view.name === "history"}
            <History />
          {:else if nav.view.name === "collection"}
            <Collection id={nav.view.id} />
          {:else if nav.view.name === "artist"}
            <Artist id={nav.view.id} />
          {:else if nav.view.name === "page"}
            <BrowsePage browseId={nav.view.browseId} params={nav.view.params ?? null} title={nav.view.title} />
          {:else if nav.view.name === "settings"}
            <Settings />
          {/if}
        </div>
      {/key}
    </div>
  </main>
  {#if ui.stageOpen}<Stage />{/if}
  <Deck />
</div>

{#if ui.theater && player.current}<Theater />{/if}
{#if ui.palette}<Palette />{/if}
{#if ui.sheet}{#key ui.sheet}<PlaylistDialog sheet={ui.sheet} />{/key}{/if}
<Overlays />
{#if customChrome}<WindowControls />{/if}

<style>
  .app {
    height: 100vh;
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr) var(--stage-w);
    grid-template-rows: minmax(0, 1fr) var(--deck-h);
    grid-template-areas:
      "side main stage"
      "deck deck deck";
  }
  .app.stage-closed {
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    grid-template-areas:
      "side main"
      "deck deck";
  }
  main {
    grid-area: main;
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  .scroll {
    height: 100%;
    overflow-y: auto;
    padding-top: var(--top-h);
    position: relative;
  }
  .view {
    animation: in 0.28s var(--ease);
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin: 8px var(--pad, 32px) 0;
    padding: 12px 16px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--accent) 14%, var(--panel));
    font-size: 13.5px;
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  @media (max-width: 1000px) {
    .app {
      --sidebar-w: 76px;
      --stage-w: 320px;
    }
  }
</style>
