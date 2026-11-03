<script lang="ts">
  import { untrack } from "svelte";
  import { saved, type NameSheet } from "$lib/state.svelte";

  // One name, for a folder or a saved queue. Enter saves, Escape cancels,
  // focus stays inside, and the sheet closes only once the name is kept.
  let { sheet: given }: { sheet: NameSheet } = $props();
  const sheet = untrack(() => given);

  let name = $state(sheet.value);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let touched = $state(false);
  const trimmed = $derived(name.trim());
  const invalid = $derived(!trimmed ? "Give it a name." : trimmed.length > 120 ? "Names can be up to 120 characters." : null);
  const note = $derived(sheet.note?.(name) ?? null);

  let box = $state<HTMLDivElement>();
  let input = $state<HTMLInputElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    input?.focus();
    input?.select();
    return () => opener?.isConnected && opener.focus();
  });

  const close = () => {
    if (!busy) saved.naming = null;
  };

  function key(e: KeyboardEvent) {
    // The app's shortcuts (Space, letters) stay out of the sheet.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Tab" && box) {
      const f = [...box.querySelectorAll<HTMLElement>("input, button:not(:disabled)")];
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) (f.at(-1)?.focus(), e.preventDefault());
      else if (!e.shiftKey && i === f.length - 1) (f[0]?.focus(), e.preventDefault());
    }
  }

  async function submit(e: Event) {
    e.preventDefault();
    touched = true;
    if (invalid || busy) return;
    busy = true;
    error = null;
    try {
      await sheet.run(trimmed);
      saved.naming = null;
    } catch (err: any) {
      error = err?.message ?? String(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="name-title" tabindex="-1" bind:this={box} onkeydown={key}>
    <h2 id="name-title">{sheet.title}</h2>
    {#if sheet.lead}<p class="lead">{sheet.lead}</p>{/if}
    <form onsubmit={submit}>
      <label class="field">
        <span>Name</span>
        <input bind:this={input} bind:value={name} maxlength="130" autocomplete="off" spellcheck="false" aria-invalid={touched && !!invalid} />
      </label>
      {#if touched && invalid}<p class="err" role="alert">{invalid}</p>{:else if note}<p class="note">{note}</p>{/if}
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="btns">
        <button type="button" class="pill" disabled={busy} onclick={close}>Cancel</button>
        <button type="submit" class="pill primary" disabled={busy || (touched && !!invalid)}>{sheet.confirm}</button>
      </div>
    </form>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 55;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.4);
    animation: fade 0.15s var(--ease);
  }
  .sheet {
    width: min(400px, calc(100vw - 48px));
    padding: 22px 24px 20px;
    border-radius: 16px;
    background: var(--raise);
    box-shadow: var(--shadow);
    animation: pop 0.16s var(--ease);
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
    text-wrap: pretty;
  }
  form {
    display: grid;
    gap: 6px;
    margin-top: 16px;
  }
  .field {
    display: grid;
    gap: 6px;
  }
  .field > span {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--mute);
  }
  input {
    width: 100%;
    padding: 9px 12px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: var(--panel);
    color: var(--text);
    font: inherit;
    font-size: 14px;
    -webkit-user-select: text;
    user-select: text;
  }
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    border-color: transparent;
  }
  .note {
    font-size: 12.5px;
    color: var(--mute);
  }
  .err {
    font-size: 12.5px;
    color: hsl(0 70% 62%);
  }
  :global([data-theme="light"]) .err {
    color: hsl(0 65% 45%);
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
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
