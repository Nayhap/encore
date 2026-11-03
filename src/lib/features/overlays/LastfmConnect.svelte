<script lang="ts">
  // Connect Last.fm (D31): the user's own API key and secret, then Last.fm's
  // approval page in the browser, then Finish. Two steps on one sheet.
  import type { BackendError } from "$lib/backend";
  import { isMac, os } from "$lib/platform";
  import { LASTFM_CREATE, lastfm, openExternal } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";

  const store = isMac ? "your Keychain" : os === "windows" ? "Windows Credential Manager" : "your keyring";
  let apiKey = $state("");
  let secret = $state("");
  let url = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  const approving = $derived(!!url && !!lastfm.status?.approving);
  const ready = $derived(apiKey.trim().length > 0 && secret.trim().length > 0);

  let box = $state<HTMLDivElement>();
  let first = $state<HTMLInputElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    first?.focus();
    return () => opener?.isConnected && opener.focus();
  });

  async function run(f: () => Promise<unknown>) {
    busy = true;
    error = null;
    try {
      await f();
    } catch (e) {
      error = (e as BackendError).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  const begin = (e: Event) => {
    e.preventDefault();
    if (ready && !busy) void run(async () => (url = await lastfm.begin(apiKey, secret)));
  };

  function close() {
    if (!busy) lastfm.cancel();
  }

  function key(e: KeyboardEvent) {
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
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="fm-title" tabindex="-1" bind:this={box} onkeydown={key}>
    <h2 id="fm-title">Connect Last.fm</h2>
    {#if !approving}
      <p class="lead">Connect with your Last.fm API key.</p>
      <ol>
        <li>
          <span>Create an API account on Last.fm. Any name and description will do; leave the callback URL empty.</span>
          <button type="button" class="link" onclick={() => openExternal(LASTFM_CREATE)}>Open Last.fm <Icon name="external" size={13} /></button>
        </li>
        <li><span>Copy its API key and shared secret here.</span></li>
      </ol>
      <form onsubmit={begin}>
        <label class="field">
          <span>API key</span>
          <input bind:this={first} bind:value={apiKey} class="mono" maxlength="64" placeholder="32 letters and digits" autocomplete="off" spellcheck="false" />
        </label>
        <label class="field">
          <span>Shared secret</span>
          <input bind:value={secret} type="password" class="mono" maxlength="64" placeholder="32 letters and digits" autocomplete="off" spellcheck="false" />
        </label>
        {#if error}<p class="err" role="alert">{error}</p>{/if}
        <p class="fine">Kept in {store}. Plays from this computer are sent, whichever YouTube account is signed in; plays in a private session aren't.</p>
        <div class="btns">
          <button type="button" class="pill" disabled={busy} onclick={close}>Cancel</button>
          <button type="submit" class="pill primary" disabled={!ready || busy}>{busy ? "Checking…" : "Continue in browser"}</button>
        </div>
      </form>
    {:else}
      <div class="wait">
        <span class="pulse" aria-hidden="true"><Icon name="external" size={18} /></span>
        <p>Last.fm is open in your browser. Choose <b>Yes, allow access</b> there, then come back and finish.</p>
      </div>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="btns">
        <button type="button" class="pill" disabled={busy} onclick={() => url && openExternal(url)}>Open the page again</button>
        <button type="button" class="pill primary" disabled={busy} onclick={() => run(() => lastfm.finish())}>{busy ? "Finishing…" : "Finish"}</button>
      </div>
      <button type="button" class="back" disabled={busy} onclick={() => ((url = null), (error = null))}><Icon name="left" size={14} />Use another key</button>
    {/if}
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
    width: min(460px, calc(100vw - 48px));
    padding: 22px 24px 20px;
    border-radius: 16px;
    background: var(--raise);
    box-shadow: var(--shadow);
    animation: pop 0.16s var(--ease);
    display: grid;
    gap: 12px;
  }
  .sheet:focus {
    outline: none;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .lead,
  .fine {
    font-size: 13px;
    color: var(--mute);
  }
  .fine {
    margin-top: 4px;
    font-size: 12.5px;
  }
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: step;
    display: grid;
    gap: 8px;
  }
  li {
    counter-increment: step;
    display: grid;
    grid-template-columns: 22px 1fr;
    column-gap: 10px;
    font-size: 13.5px;
  }
  li::before {
    content: counter(step);
    grid-row: span 2;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font: 700 11.5px/1 var(--font-mono);
    background: var(--accent-soft);
    color: var(--accent);
  }
  li .link {
    justify-self: start;
    margin-top: 3px;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .link:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  form {
    display: grid;
    gap: 8px;
    margin-top: 4px;
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
    font-size: 13.5px;
    -webkit-user-select: text;
    user-select: text;
  }
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    border-color: transparent;
  }
  input::placeholder {
    color: var(--dim);
    font-family: var(--font-ui);
  }
  .err {
    font-size: 13px;
    color: hsl(0 70% 62%);
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  .wait {
    display: flex;
    gap: 14px;
    align-items: center;
    padding: 14px;
    border-radius: 12px;
    background: var(--hover);
    font-size: 13.5px;
  }
  .pulse {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    flex: none;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    animation: pulse 1.8s var(--ease) infinite;
  }
  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 8px transparent, 0 0 0 0 var(--accent-soft);
      transform: scale(0.94);
    }
  }
  .back {
    justify-self: start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12.5px;
    color: var(--mute);
  }
  .back:hover {
    color: var(--text);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }
</style>
