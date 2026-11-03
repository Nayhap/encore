<script lang="ts">
  import { onMount } from "svelte";
  import { call } from "$lib/backend";
  import { account, openExternal, player, setup, taste, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { mb, settings } from "./shared.svelte";

  const busy = $derived(settings.busy);
  // The blocked songs and artists (D36), listed on request.
  let showBlocked = $state(false);
  const blockedCount = $derived(taste.count === 1 ? "1 song or artist" : `${taste.count} songs and artists`);

  onMount(async () => {
    // Measured once here: the size of the components folder.
    const rt = await call((a) => a.runtimesStatus()).catch(() => null);
    if (rt) setup.apply(rt);
  });

  function setReport(on: boolean) {
    ui.reportHistory = on;
    call((a) => a.setReportHistory(on)).catch((e) => {
      ui.reportHistory = !on;
      ui.say(`Could not change that: ${e.message}`);
    });
  }
  const rt = $derived(setup.status);
  const versions = $derived((rt?.components ?? []).map((c) => `${c.name} ${c.installed ?? c.version}`).join(" · "));
  function removeComponents() {
    ui.confirm = {
      title: "Remove player components?",
      text: `This frees about ${mb(rt?.usedBytes || rt?.diskBytes || 0)}. Streaming and new downloads need them, so they wait until you set up playback again. Downloaded songs still play.`,
      confirm: "Remove",
      danger: true,
      run: () => void settings.run("runtimes", () => setup.remove()),
    };
  }
</script>

<section>
  <h2>Playback</h2>
  <label class="row">
    <div>
      <b>Autoplay</b>
      <p>Keep playing similar songs when the queue ends. Off: only what you choose plays.</p>
    </div>
    <input type="checkbox" role="switch" checked={player.autoplay} onchange={(e) => player.setAutoplay(e.currentTarget.checked)} />
  </label>
  <div class="row">
    <label class="grow" for="report-history">
      <b>Send plays to YouTube history</b>
      <p>
        {account.signedIn ? "Songs you play here show up in your YouTube Music history and shape your recommendations." : "When you're signed in, songs you play here show up in your YouTube Music history."}
        Plays while you're offline aren't sent. YouTube's own history setting still applies.
      </p>
      <span class="links">
        <button type="button" class="link" onclick={() => openExternal("https://myactivity.google.com/product/youtube")}>Manage your YouTube history <Icon name="external" size={13} /></button>
        {#if account.signedIn}
          {#if taste.private}
            <button type="button" class="link" onclick={() => taste.setPrivate(false)}><Icon name="lock" size={13} /> End private session{taste.privateLeftText ? ` (${taste.privateLeftText})` : ""}</button>
          {:else}
            <button type="button" class="link" onclick={() => taste.setPrivate(true)}><Icon name="lock" size={13} /> Start a private session</button>
          {/if}
        {/if}
      </span>
    </label>
    <input id="report-history" type="checkbox" role="switch" checked={ui.reportHistory} onchange={(e) => setReport(e.currentTarget.checked)} />
  </div>
  <div class="row">
    <div>
      <b>Blocked songs and artists{taste.count ? ` · ${taste.count}` : ""}</b>
      <p>
        {#if taste.count}Encore never plays {taste.count === 1 ? "it" : "these"} on its own, and leaves {taste.count === 1 ? "it" : "them"} out of Home and Explore.
        {:else}Choose “Don't play this again” in a song's menu, or “Don't play this artist” on an artist's page.{/if}
      </p>
    </div>
    {#if taste.count}
      <button class="pill" aria-expanded={showBlocked} onclick={() => (showBlocked = !showBlocked)}>{showBlocked ? "Hide" : `Show ${blockedCount}`}</button>
    {/if}
  </div>
  {#if showBlocked && taste.count}
    <ul class="blocked" aria-label="Blocked songs and artists">
      {#each taste.blocked as b (b.kind + b.target)}
        <li>
          <Icon name={b.kind === "artist" ? "artist" : "album"} size={16} />
          <span><b>{b.label}</b>{#if b.detail}<small>{" · "}{b.detail}</small>{/if}{#if b.kind === "artist"}<small>{" · "}Artist</small>{/if}</span>
          <button class="pill" onclick={() => taste.unblock(b.kind, b.target)}>Play again</button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="row">
    <div>
      <b>Player components</b>
      <p>
        {#if !rt}Checking…
        {:else if !rt.supported}{rt.message}
        {:else if rt.installing}{rt.usable ? "Updating" : "Setting up"} · {Math.floor((rt.progress?.fraction ?? 0) * 100)}%
        {:else if rt.usable}{versions}{rt.usedBytes ? ` · ${mb(rt.usedBytes)}` : ""}{rt.ready ? "" : " · an update is waiting"}
        {:else}Not installed. Playback needs Python, Deno and yt-dlp, about {mb(rt.downloadBytes ?? 0)} to download.{/if}
      </p>
    </div>
    <div class="btns">
      {#if rt?.installing}
        <button class="pill" onclick={() => setup.show()}>Show progress</button>
      {:else if rt?.usable}
        <button class="pill" disabled={!!busy} onclick={() => setup.install(true).then(() => setup.show())}>Reinstall components</button>
        <button class="pill" disabled={!!busy} onclick={removeComponents}>Remove downloaded components</button>
      {:else if rt?.supported}
        <button class="pill primary" onclick={() => setup.show()}>Set up playback</button>
      {/if}
    </div>
  </div>
</section>

<style>
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 0 16px;
  }
  .blocked {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 8px 0;
    list-style: none;
    border-bottom: 1px solid var(--line);
  }
  .blocked li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 4px 6px 10px;
    border-radius: 10px;
    font-size: 13.5px;
  }
  .blocked li:hover {
    background: var(--hover);
  }
  .blocked li > :global(svg) {
    flex: none;
    color: var(--mute);
  }
  .blocked span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .blocked b {
    font-weight: 600;
  }
  .blocked small {
    font-size: 13px;
    color: var(--mute);
  }
  .blocked .pill {
    flex: none;
    height: 28px;
    font-size: 12.5px;
  }
</style>
