<script lang="ts">
  import { onMount } from "svelte";
  import { call } from "$lib/backend";
  import { account, openExternal, player, setup, ui } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { mb, settings } from "./shared.svelte";

  const busy = $derived(settings.busy);

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
      <p>Keep playing similar songs when the queue ends.</p>
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
      <button type="button" class="link" onclick={() => openExternal("https://myactivity.google.com/product/youtube")}>Manage your YouTube history <Icon name="external" size={13} /></button>
    </label>
    <input id="report-history" type="checkbox" role="switch" checked={ui.reportHistory} onchange={(e) => setReport(e.currentTarget.checked)} />
  </div>
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
