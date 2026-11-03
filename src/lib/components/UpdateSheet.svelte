<script lang="ts">
  import { openExternal, updates } from "$lib/state.svelte";
  import { isMac } from "$lib/platform";
  import Icon from "./Icon.svelte";

  // A new version of Encore (D24): its notes as plain bullets, and the one
  // thing to do about it. After an update, the same sheet says what's new.

  const st = $derived(updates.status);
  const whatsNew = $derived(updates.sheet === "whatsNew");
  const info = $derived(whatsNew ? st?.updated : st?.available);
  const phase = $derived(st?.phase);
  const date = $derived(info?.date ? new Date(info.date).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" }) : null);
  const pct = $derived(st?.total ? Math.floor(((st.received ?? 0) / st.total) * 100) : null);

  let box = $state<HTMLDivElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    phase;
    queueMicrotask(() => box?.querySelector<HTMLElement>("[data-autofocus]")?.focus());
  });
  $effect(() => () => opener?.isConnected && opener.focus());
  // Nothing left to show (skipped elsewhere, or installed meanwhile).
  $effect(() => {
    if (!info) updates.sheet = null;
  });

  const close = () => (updates.sheet = null);

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Tab" && box) {
      const f = [...box.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) (f.at(-1)?.focus(), e.preventDefault());
      else if (!e.shiftKey && i === f.length - 1) (f[0]?.focus(), e.preventDefault());
    }
  }
</script>

{#if info && st}
  <div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="update-title" tabindex="-1" bind:this={box} onkeydown={key}>
      <div class="badge" aria-hidden="true"><Icon name={whatsNew ? "check" : "download"} size={22} /></div>
      <h2 id="update-title">{whatsNew ? `What's new in Encore ${info.version}` : `Encore ${info.version}`}</h2>
      {#if date}<p class="lead">Released {date}</p>{/if}
      {#if info.notes.length}
        <ul class="notes">
          {#each info.notes as n}<li>{n}</li>{/each}
        </ul>
      {/if}
      <button type="button" class="link" onclick={() => openExternal(info.page)}>Full release notes <Icon name="external" size={13} /></button>
      {#if !whatsNew}
        {#if !st.canInstall}
          <p class="note">{st.reason}</p>
        {:else if phase === "downloading"}
          <p class="note">Downloading{pct != null ? ` · ${pct}%` : "…"}. You can keep listening.</p>
        {:else if phase === "ready"}
          <p class="note">Encore pauses, installs the update and opens again where you were.{isMac ? " macOS may ask once for access to your saved sign-in." : ""}</p>
        {:else if st.error}
          <p class="note">{st.error}</p>
        {/if}
      {/if}
      <div class="btns">
        {#if whatsNew}
          <button class="pill primary" data-autofocus onclick={close}>Done</button>
        {:else}
          {#if st.skipped !== info.version}<button class="pill quiet" onclick={() => updates.skip(info.version)}>Skip this version</button>{/if}
          <button class="pill" onclick={close}>Later</button>
          {#if !st.canInstall}
            <button class="pill primary" data-autofocus onclick={() => openExternal(info.page)}>Open releases page</button>
          {:else if phase === "ready"}
            <button class="pill primary" data-autofocus onclick={() => updates.install()}>Restart to update</button>
          {:else if phase === "available" || phase === "failed"}
            <button class="pill primary" data-autofocus onclick={() => updates.download()}>Download</button>
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 55;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.42);
    animation: fade 0.15s var(--ease);
  }
  .sheet {
    width: min(460px, calc(100vw - 48px));
    max-height: calc(100vh - 80px);
    overflow-y: auto;
    padding: 24px 26px 22px;
    border-radius: 18px;
    background: var(--raise);
    box-shadow: var(--shadow);
    animation: pop 0.16s var(--ease);
  }
  .badge {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    margin-bottom: 14px;
    border-radius: 13px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  h2 {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .lead {
    margin-top: 4px;
    font-size: 13px;
    color: var(--mute);
  }
  .notes {
    display: grid;
    gap: 7px;
    margin: 16px 0 0;
    padding: 0 0 0 18px;
    font-size: 13.5px;
    line-height: 1.45;
  }
  .notes li::marker {
    color: var(--dim);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 12px;
    font-size: 13px;
    font-weight: 550;
    color: var(--text);
  }
  .link:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .note {
    margin-top: 14px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--mute);
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 20px;
  }
  .quiet {
    margin-right: auto;
    background: none;
    color: var(--mute);
  }
  .quiet:hover {
    color: var(--text);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>
