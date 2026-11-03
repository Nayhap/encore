<script lang="ts">
  // Settings → Connections (lane E): Last.fm and Discord (D31). The rows use
  // the `.row` / `.btns` styles of the other sections.
  import { lastfm, openExternal, presence } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";

  const fm = $derived(lastfm.status);
  const waiting = $derived.by(() => {
    const out: string[] = [];
    if (fm?.queued) out.push(fm.queued === 1 ? "1 play waiting to be sent" : `${fm.queued} plays waiting to be sent`);
    if (fm?.failed) out.push(fm.failed === 1 ? "1 play Last.fm refused" : `${fm.failed} plays Last.fm refused`);
    return out.join(" · ");
  });
  const discord = $derived(presence.status);
</script>

<section>
  <h2>Connections</h2>
  <div class="row">
    <div class="grow">
      <b>Last.fm</b>
      {#if fm?.error}
        <p class="note error">{fm.error}</p>
      {:else if fm?.username}
        <p>Sending your plays to <b class="who">{fm.username}</b>{waiting ? ` · ${waiting}` : ""}</p>
        <button type="button" class="link" onclick={() => openExternal(`https://www.last.fm/user/${encodeURIComponent(fm.username!)}`)}>Open your Last.fm profile <Icon name="external" size={13} /></button>
      {:else}
        <p>Send your plays to your Last.fm profile, and sort playlists by your Last.fm plays.</p>
      {/if}
    </div>
    <div class="btns">
      {#if fm?.username}
        {#if fm.error}<button class="pill primary" onclick={() => (lastfm.sheet = true)}>Connect again</button>{/if}
        <button class="pill" onclick={() => lastfm.disconnect()}>Disconnect</button>
      {:else}
        <button class="pill" onclick={() => (lastfm.sheet = true)}>{fm?.approving ? "Finish connecting…" : "Connect…"}</button>
      {/if}
    </div>
  </div>
  {#if discord?.available}
    <div class="row">
      <label class="grow" for="discord-presence">
        <b>Show what you're playing on Discord</b>
        <p>
          The song and cover appear on your profile while Discord is open, except when paused or in a private session.
          {#if discord.enabled && !discord.connected}<span class="state">Discord isn't open.</span>{/if}
        </p>
      </label>
      <input id="discord-presence" type="checkbox" role="switch" checked={discord.enabled} onchange={(e) => presence.set(e.currentTarget.checked)} />
    </div>
  {/if}
</section>

<style>
  .who {
    font-weight: 600;
    color: var(--text);
  }
  .state {
    display: block;
    margin-top: 4px;
    color: var(--dim);
  }
</style>
