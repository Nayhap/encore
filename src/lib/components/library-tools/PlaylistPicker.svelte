<script lang="ts">
  // "Add to playlists" for a selection (D34): tick any of your playlists,
  // then one add per playlist (D20, duplicates checked), each with its own
  // answer. An add whose outcome is unknown is never sent again by itself.
  //
  // With `folder`, the same sheet fills a folder (D34) from the folder's
  // page: tick playlists and albums from your library to move them in.
  import { untrack } from "svelte";
  import { call, type BackendError, type Folder, type MutationOutcome } from "$lib/backend";
  import { catalog } from "$lib/catalog.svelte";
  import { kindLabel } from "$lib/data";
  import { fold } from "./logic";
  import { fileIn, newPlaylist, playlistPicker, playlists, saved, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";

  let { folder, onclose }: { folder?: Folder; onclose?: () => void } = $props();

  const ids = untrack(() => (folder ? [] : [...playlistPicker.trackIds]));
  const exclude = untrack(() => (folder ? null : playlistPicker.exclude));
  const bare = (id: string) => id.replace(/^(ytm|mock):/, "").replace(/^VL/, "");
  const songs = (n: number) => (n === 1 ? "1 song" : `${n.toLocaleString()} songs`);
  const one = ids.length === 1 ? catalog.track(ids[0])?.title : null;

  if (!playlists.loaded) void playlists.load();
  const lists = $derived(
    folder
      ? playlists.list.filter((p) => !folder.items.some((i) => i.itemId === p.id))
      : playlists.list.filter((p) => p.kind === "playlist" && bare(p.id) !== (exclude ? bare(exclude) : "")),
  );
  let q = $state("");
  const shown = $derived(q.trim() ? lists.filter((p) => fold(p.title).includes(fold(q))) : lists);

  type Outcome = "adding" | MutationOutcome | "error";
  const picked = $state<Record<string, boolean>>({});
  const outcome = $state<Record<string, Outcome>>({});
  const errors: Record<string, string> = {};
  let running = $state(false);
  let ran = $state(false);
  const count = $derived(Object.values(picked).filter(Boolean).length);

  const close = () => {
    if (running) return;
    if (folder) return onclose?.();
    playlistPicker.open = false;
    const added = Object.values(outcome).filter((o) => o === "done").length;
    if (added) ui.say(`Added ${one ? `“${one}”` : songs(ids.length)} to ${added === 1 ? "1 playlist" : `${added} playlists`}`);
  };

  async function addTo(id: string, allowDuplicates: boolean) {
    outcome[id] = "adding";
    try {
      outcome[id] = await call((a) => a.playlistAdd(bare(id), ids, allowDuplicates));
    } catch (e) {
      outcome[id] = "error";
      errors[id] = (e as BackendError).message;
    }
  }

  async function run() {
    if (ran) return close();
    if (!count) return;
    running = true;
    if (folder) {
      const todo = lists.filter((p) => picked[p.id]);
      for (const p of todo) await fileIn(folder, p.id, true);
      running = false;
      ui.say(todo.length === 1 ? `Added “${todo[0].title}” to “${folder.name}”` : `Added ${todo.length} to “${folder.name}”`);
      return onclose?.();
    }
    const todo = lists.filter((p) => picked[p.id]).map((p) => p.id);
    for (const [i, id] of todo.entries()) {
      await addTo(id, false);
      // A short gap between writes; YouTube sees one person (D23).
      if (i < todo.length - 1) await new Promise((r) => setTimeout(r, 600));
    }
    running = false;
    ran = true;
  }

  const said: Record<Outcome, string> = {
    adding: "Adding…",
    done: "Added",
    duplicate: ids.length === 1 ? "Already in it" : "Has some already",
    rejected: "Not accepted",
    outcomeUnknown: "Not sure it worked",
    error: "Couldn't add",
  };
  const hint = (id: string, o: Outcome) =>
    o === "outcomeUnknown" ? "Check the playlist before adding again" : o === "error" ? errors[id] : o === "rejected" ? "YouTube Music refused this add" : o === "duplicate" ? "Nothing was added to this one" : undefined;

  let box = $state<HTMLDivElement>();
  let find = $state<HTMLInputElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    find?.focus();
    return () => opener?.isConnected && opener.focus();
  });
  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (q) q = "";
      else close();
    } else if (e.key === "Tab" && box) {
      const f = [...box.querySelectorAll<HTMLElement>("input, button:not(:disabled)")];
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) (f.at(-1)?.focus(), e.preventDefault());
      else if (!e.shiftKey && i === f.length - 1) (f[0]?.focus(), e.preventDefault());
    }
  }
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="pp-title" tabindex="-1" bind:this={box} onkeydown={key}>
    {#if folder}
      <h2 id="pp-title">Add to “{folder.name}”</h2>
      <p class="lead">Tick playlists and albums to move them into this folder</p>
    {:else}
      <h2 id="pp-title">Add to playlists</h2>
      <p class="lead">{one ? `“${one}”` : songs(ids.length)}{ran ? "" : ", to every playlist you tick"}</p>
    {/if}
    {#if lists.length > 6}
      <label class="find">
        <Icon name="search" size={16} />
        <input bind:this={find} bind:value={q} placeholder="Find a playlist" spellcheck="false" autocomplete="off" />
      </label>
    {/if}
    <ul class="lists" aria-label="Your playlists">
      {#if !ran && !folder}
        <li>
          <button class="row new" onclick={() => ((playlistPicker.open = false), newPlaylist(ids))}>
            <span class="plus"><Icon name="plus" size={18} /></span>
            <span class="txt"><b>New playlist…</b><small>Starts with {one ? "this song" : "these songs"}</small></span>
          </button>
        </li>
      {/if}
      {#each shown as p (p.id)}
        {@const o = outcome[p.id]}
        {@const home = folder ? saved.folderOf(p.id) : undefined}
        <li>
          <button class="row" class:on={picked[p.id]} role="checkbox" aria-checked={!!picked[p.id]} disabled={running || ran} onclick={() => (picked[p.id] = !picked[p.id])}>
            <span class="box" class:on={picked[p.id]}>{#if picked[p.id]}<Icon name="check" size={13} stroke={2.6} />{/if}</span>
            <span class="art"><Cover art={p.art} radius={6} size={120} label={p.title} /></span>
            <span class="txt"><b class="ell">{p.title}</b>{#if o && hint(p.id, o)}<small class="ell">{hint(p.id, o)}</small>{:else if folder}<small class="ell">{home ? `In “${home.name}”` : kindLabel[p.kind]}</small>{/if}</span>
            {#if o}<span class="out {o}">{#if o === "done"}<Icon name="check" size={14} />{/if}{said[o]}</span>{/if}
          </button>
          {#if o === "duplicate"}
            <button class="again" onclick={() => void addTo(p.id, true)}>Add anyway</button>
          {/if}
        </li>
      {:else}
        <li class="none">{q ? "No playlist by that name." : !playlists.loaded ? "Loading your playlists…" : folder ? "Everything in your library is in this folder already." : "You have no other playlists yet."}</li>
      {/each}
    </ul>
    <div class="btns">
      {#if !ran}<button class="pill" disabled={running} onclick={close}>Cancel</button>{/if}
      <button class="pill primary" disabled={running || (!ran && !count)} onclick={run}>
        {running ? "Adding…" : ran ? "Done" : folder ? (count > 1 ? `Add ${count}` : "Add") : count > 1 ? `Add to ${count} playlists` : "Add"}
      </button>
    </div>
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
    display: grid;
    grid-template-rows: auto auto auto minmax(0, 1fr) auto;
    width: min(460px, calc(100vw - 48px));
    max-height: min(640px, calc(100vh - 64px));
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
  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 14px;
    padding: 0 12px;
    height: 38px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: var(--panel);
    color: var(--mute);
  }
  .find:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    border-color: transparent;
  }
  .find input {
    flex: 1;
    border: 0;
    background: none;
    font-size: 14px;
    -webkit-user-select: text;
    user-select: text;
  }
  .find input:focus-visible {
    outline: none;
  }
  .lists {
    list-style: none;
    margin: 12px -8px 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
  }
  .lists li {
    position: relative;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 52px;
    padding: 6px 8px;
    border-radius: 10px;
    text-align: left;
  }
  .row:hover:not(:disabled) {
    background: var(--hover);
  }
  .row.on {
    background: var(--accent-soft);
  }
  .row:disabled {
    cursor: default;
  }
  .box {
    display: grid;
    place-items: center;
    flex: none;
    width: 18px;
    height: 18px;
    border-radius: 5px;
    box-shadow: inset 0 0 0 1.5px var(--dim);
    color: var(--accent-ink);
  }
  .box.on {
    background: var(--accent);
    box-shadow: none;
  }
  .art,
  .plus {
    flex: none;
    width: 38px;
    height: 38px;
  }
  .plus {
    display: grid;
    place-items: center;
    border-radius: 6px;
    background: var(--hover);
    color: var(--text);
    margin-left: 30px;
  }
  .txt {
    display: grid;
    min-width: 0;
    flex: 1;
  }
  .txt b {
    font-size: 13.5px;
    font-weight: 600;
  }
  .txt small {
    font-size: 12px;
    color: var(--mute);
  }
  .out {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--mute);
  }
  .out.done {
    color: var(--accent);
  }
  .out.rejected,
  .out.error {
    color: hsl(0 70% 62%);
  }
  :global([data-theme="light"]) .out.rejected,
  :global([data-theme="light"]) .out.error {
    color: hsl(0 65% 45%);
  }
  .again {
    position: absolute;
    right: 8px;
    bottom: 4px;
    font-size: 12px;
    font-weight: 650;
    color: var(--accent);
  }
  .again:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .lists li:has(.again) .out {
    transform: translateY(-8px);
  }
  .none {
    padding: 14px 8px;
    font-size: 13px;
    color: var(--mute);
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
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
