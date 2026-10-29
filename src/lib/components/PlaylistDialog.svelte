<script lang="ts">
  import { untrack } from "svelte";
  import { call, type BackendError, type Collection, type PlaylistEdit, type Privacy } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { nav, playlists, ui, type PlaylistSheet } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  // Create or edit a playlist (D20). New playlists are private unless the
  // user picks otherwise; songs from "Add to playlist" go in with the create.
  let { sheet: given }: { sheet: PlaylistSheet } = $props();
  // The sheet is opened once per request (keyed in App); later changes to
  // the prop don't apply.
  const sheet = untrack(() => given);

  const editing = sheet.mode === "edit";
  let title = $state(sheet.title ?? "");
  let description = $state(sheet.description ?? "");
  let privacy = $state<Privacy>(sheet.privacy ?? "private");
  let busy = $state(false);
  let error = $state<string | null>(null);

  const options: { v: Privacy; label: string; hint: string; icon: string }[] = [
    { v: "private", label: "Private", hint: "Only you can see it", icon: "lock" },
    { v: "unlisted", label: "Unlisted", hint: "Anyone with the link can see it", icon: "link" },
    { v: "public", label: "Public", hint: "Anyone can find and see it", icon: "globe" },
  ];

  const songs = sheet.trackIds ?? [];
  const firstSong = songs.length ? catalog.track(songs[0])?.title : null;
  const startsWith = songs.length === 1 && firstSong ? `Starts with “${firstSong}”.` : songs.length > 1 ? `Starts with ${songs.length} songs.` : null;

  const trimmed = $derived(title.trim());
  const titleError = $derived(!trimmed ? "Give the playlist a name." : /[<>]/.test(trimmed) ? "Names can't contain < or >." : trimmed.length > 150 ? "Names can be up to 150 characters." : null);
  const descError = $derived(/[<>]/.test(description) ? "Descriptions can't contain < or >." : description.length > 5000 ? "Descriptions can be up to 5000 characters." : null);
  const edit = $derived.by<PlaylistEdit>(() => ({
    title: trimmed !== (sheet.title ?? "") ? trimmed : null,
    description: description.trim() !== (sheet.description ?? "").trim() ? description.trim() : null,
    privacy: privacy !== (sheet.privacy ?? null) ? privacy : null,
  }));
  const changed = $derived(!editing || edit.title != null || edit.description != null || edit.privacy != null);
  const canSubmit = $derived(!busy && !titleError && !descError && changed);
  let touched = $state(false);

  let box = $state<HTMLDivElement>();
  let titleEl = $state<HTMLInputElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    titleEl?.focus();
    if (editing) titleEl?.select();
    return () => opener?.isConnected && opener.focus();
  });

  const close = () => {
    if (!busy) ui.sheet = null;
  };

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "Tab" && box) {
      // Keep focus inside the sheet.
      const f = [...box.querySelectorAll<HTMLElement>("input, textarea, button:not(:disabled)")];
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) (f.at(-1)?.focus(), e.preventDefault());
      else if (!e.shiftKey && i === f.length - 1) (f[0]?.focus(), e.preventDefault());
    }
  }

  async function submit(e: Event) {
    e.preventDefault();
    touched = true;
    if (!canSubmit) return;
    busy = true;
    error = null;
    try {
      if (editing) await save();
      else await create();
    } catch (err) {
      error = (err as BackendError).message ?? String(err);
    } finally {
      busy = false;
    }
  }

  async function create() {
    const r = await call((a) => a.playlistCreate(trimmed, description.trim(), privacy, songs));
    if (r.outcome === "done" && r.id) {
      playlists.insert({ id: r.id, kind: "playlist", title: trimmed, subtitle: "Playlist", artists: [], year: null, art: null, playId: null });
      ui.sheet = null;
      if (songs.length) ui.say(`Added to “${trimmed}”`);
      else {
        ui.say(`Created “${trimmed}”`);
        nav.go({ name: "collection", id: r.id });
      }
    } else if (r.outcome === "outcomeUnknown") {
      // Never create again on our own: it may already exist.
      ui.sheet = null;
      ui.say("Could not confirm the playlist was created. Check your playlists before trying again.");
      playlists.changed();
    } else {
      error = "YouTube Music didn't create the playlist. Try again in a moment.";
    }
  }

  async function save() {
    const id = sheet.id!;
    const r = await call((a) => a.playlistEdit(sheet.playlistId!, edit));
    if (r === "done") {
      const c = catalog.collection(id);
      if (c && "editable" in c) catalog.collections.set(id, { ...(c as Collection), title: trimmed, description: description.trim() || null, privacy });
      playlists.list = playlists.list.map((p) => (p.id === id ? { ...p, title: trimmed } : p));
      ui.sheet = null;
      ui.say("Saved");
    } else if (r === "outcomeUnknown") {
      ui.sheet = null;
      ui.say("Could not confirm the changes were saved. Reloading the playlist.");
      playlists.changed();
    } else {
      error = "YouTube Music didn't accept those changes. Try again in a moment.";
    }
  }
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="pl-title" tabindex="-1" bind:this={box} onkeydown={key}>
    <h2 id="pl-title">{editing ? "Edit details" : "New playlist"}</h2>
    {#if startsWith}<p class="lead">{startsWith}</p>{/if}
    <form onsubmit={submit}>
      <label class="field">
        <span>Name</span>
        <input bind:this={titleEl} bind:value={title} maxlength="160" placeholder="My playlist" autocomplete="off" spellcheck="false" aria-invalid={touched && !!titleError} />
      </label>
      {#if touched && titleError}<p class="err" role="alert">{titleError}</p>{/if}
      <label class="field">
        <span>Description</span>
        <textarea bind:value={description} rows="3" placeholder="Optional" aria-invalid={!!descError}></textarea>
      </label>
      {#if descError}<p class="err" role="alert">{descError}</p>{/if}
      <fieldset>
        <legend>Who can see it</legend>
        <div class="opts">
          {#each options as o}
            <label class="opt" class:on={privacy === o.v}>
              <input type="radio" name="privacy" value={o.v} bind:group={privacy} />
              <Icon name={o.icon} size={17} />
              <span><b>{o.label}</b><small>{o.hint}</small></span>
            </label>
          {/each}
        </div>
      </fieldset>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="btns">
        <button type="button" class="pill" disabled={busy} onclick={close}>Cancel</button>
        <button type="submit" class="pill primary" disabled={busy || (touched && !canSubmit) || (editing && !changed)}>
          {busy ? (editing ? "Saving…" : "Creating…") : editing ? "Save" : "Create"}
        </button>
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
    width: min(440px, calc(100vw - 48px));
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
  }
  form {
    display: grid;
    gap: 6px;
    margin-top: 16px;
  }
  .field {
    display: grid;
    gap: 6px;
    margin-top: 6px;
  }
  .field > span,
  legend {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--mute);
  }
  input:not([type="radio"]),
  textarea {
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
    resize: none;
  }
  input:not([type="radio"]):focus-visible,
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    border-color: transparent;
  }
  input::placeholder,
  textarea::placeholder {
    color: var(--dim);
  }
  fieldset {
    margin: 10px 0 0;
    padding: 0;
    border: 0;
  }
  legend {
    padding: 0;
    margin-bottom: 6px;
  }
  .opts {
    display: grid;
    gap: 4px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 8px 12px;
    border-radius: 10px;
    border: 1px solid var(--line);
    color: var(--mute);
  }
  .opt:hover {
    background: var(--hover);
  }
  .opt.on {
    border-color: color-mix(in srgb, var(--accent) 70%, transparent);
    background: var(--accent-soft);
    color: var(--text);
  }
  .opt:has(input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .opt input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .opt span {
    display: grid;
  }
  .opt b {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
  }
  .opt small {
    font-size: 12px;
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
