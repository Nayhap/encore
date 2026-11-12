<script lang="ts">
  import { onMount } from "svelte";
  import { call, type EngineStatus } from "$lib/backend";
  import { account, autoplayCopy, openExternal, player, taste, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { settings } from "./shared.svelte";
  // The blocked songs and artists (D36), listed on request.
  let showBlocked = $state(false);

  // The built-in player engine (D37): versions folded away, one reset.
  let engine = $state<EngineStatus | null>(null);
  let showEngine = $state(false);
  // Asked again while Settings is open, so a notice set by a song that
  // fails meanwhile (or cleared by one that plays) shows here.
  onMount(() => {
    const read = () =>
      call((a) => a.playerEngine())
        .then((e) => (engine = e))
        .catch(() => {});
    read();
    const t = setInterval(read, 4000);
    return () => clearInterval(t);
  });
  async function resetEngine() {
    const e = await settings.run("engine", () => call((a) => a.playerEngineReset()));
    if (e) {
      engine = e;
      ui.say("Player cache reset");
    }
  }

  function setNormalise(on: boolean) {
    ui.normalise = on;
    call((a) => a.setNormalise(on)).catch((e) => {
      ui.normalise = !on;
      ui.say(`Could not change that: ${e.message}`);
    });
  }

  function setReport(on: boolean) {
    ui.reportHistory = on;
    call((a) => a.setReportHistory(on)).catch((e) => {
      ui.reportHistory = !on;
      ui.say(`Could not change that: ${e.message}`);
    });
  }
</script>

<section>
  <h2>Playback</h2>
  <label class="row">
    <div>
      <b>Autoplay</b>
      <p>{autoplayCopy(player.autoplay, ui.online)}</p>
    </div>
    <input type="checkbox" role="switch" checked={player.autoplay} onchange={(e) => player.setAutoplay(e.currentTarget.checked)} />
  </label>
  <label class="row">
    <div>
      <b>Normalise volume</b>
      <p>Turns loud songs down so every song plays at about the same level, as youtube.com does. Off, each song plays as loud as it was made, so some are louder than others. A change applies from the next song.</p>
    </div>
    <input type="checkbox" role="switch" checked={ui.normalise} onchange={(e) => setNormalise(e.currentTarget.checked)} />
  </label>
  <div class="row">
    <label class="grow" for="report-history">
      <b>Send plays to YouTube history</b>
      <p>
        {account.signedIn ? "Songs you play here show up in your YouTube Music history and shape your recommendations." : "When you're signed in, songs you play here show up in your YouTube Music history."}
        Plays while you're offline aren't sent. YouTube's own history setting still applies.
      </p>
      <button type="button" class="link" onclick={() => openExternal("https://myactivity.google.com/product/youtube")}>Manage your YouTube history <Icon name="external" size={13} /></button>
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
      <button class="pill" aria-expanded={showBlocked} onclick={() => (showBlocked = !showBlocked)}>{showBlocked ? "Hide" : "Show"}</button>
    {/if}
  </div>
  <div class="row">
    <div>
      <b>Player engine</b>
      <p class:warn={!!engine?.notice}>{engine?.notice ?? "Built into Encore. Nothing to download or set up."}</p>
    </div>
    <button class="pill more" aria-expanded={showEngine} onclick={() => (showEngine = !showEngine)}>Details<Icon name={showEngine ? "up" : "down"} size={14} /></button>
  </div>
  {#if showEngine && engine}
    <ul class="blocked engine" aria-label="Player engine">
      <li><span><b>Extractor</b><small>{" · "}{engine.extractor}</small></span></li>
      <li><span><b>Challenge solver</b><small>{" · "}{engine.solver} on {engine.engine}</small></span></li>
      <li>
        <span><b>Player script</b><small>{" · "}{engine.player ?? "not fetched yet"}</small></span>
        <button class="pill" disabled={!!settings.busy} onclick={resetEngine}>Reset player cache</button>
      </li>
    </ul>
  {/if}
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
</section>

<style>
  .pill.more {
    gap: 6px;
  }
  .warn {
    color: var(--text);
    font-weight: 550;
  }
  .engine li {
    min-height: 36px;
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
