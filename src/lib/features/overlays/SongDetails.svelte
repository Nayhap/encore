<script lang="ts">
  // Song details (D28): what Encore knows about a song, in one sheet.
  // Tempo and key are estimated on request from the song's own file, and
  // say so; a value Encore isn't sure of shows as "—".
  import { artistLine, fmt, sizeText } from "$lib/data";
  import { nav, songDetails as d, ui } from "$lib/state.svelte";
  import Cover from "$lib/components/Cover.svelte";
  import Icon from "$lib/components/Icon.svelte";

  const t = $derived(d.track!);
  const info = $derived(d.info);
  const a = $derived(info?.analysis ?? null);
  const file = $derived(info?.file ?? null);
  const where = $derived(file ? (file.downloaded ? "downloaded file" : "cached file") : null);

  const plays = $derived.by(() => {
    if (!info) return null;
    const out = [info.plays ? `${info.plays} here` : "None here yet"];
    if (info.lastfmPlays != null) out.push(`${info.lastfmPlays.toLocaleString()} on Last.fm`);
    const yt = t.plays?.replace(/\s*(plays|views)$/i, "");
    if (yt) out.push(`${yt} on YouTube`);
    return out.join(" · ");
  });
  const fileLine = $derived(file ? [file.downloaded ? "Downloaded" : "In the cache", file.format, file.kbps ? `${file.kbps} kbps` : null, sizeText(file.bytes)].filter(Boolean).join(" · ") : null);

  let box = $state<HTMLDivElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    box?.focus();
    return () => opener?.isConnected && opener.focus();
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      d.close();
    } else if (e.key === "Tab" && box) {
      const f = [...box.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) (f.at(-1)?.focus(), e.preventDefault());
      else if (!e.shiftKey && i === f.length - 1) (f[0]?.focus(), e.preventDefault());
    }
  }

  function goAlbum() {
    const id = t.album?.id;
    if (!id) return;
    d.close();
    ui.theater = false;
    nav.go({ name: "collection", id });
  }
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && d.close()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="sd-title" tabindex="-1" bind:this={box} onkeydown={key}>
    <header>
      <div class="art"><Cover art={t.art} radius={8} size={120} label={t.title} /></div>
      <div class="who">
        <h2 id="sd-title" class="ell">{t.title}</h2>
        <p class="ell">{artistLine(t)}</p>
      </div>
    </header>

    <section class="measure" aria-live="polite">
      {#if a && !d.estimating}
        <div class="tiles">
          <div class="tile" title={a.bpm == null ? "Not certain enough to say" : undefined}>
            <span class="label">Tempo</span>
            <span class="value mono">{a.bpm != null ? Math.round(a.bpm) : "—"}{#if a.bpm != null}<small>BPM</small>{/if}</span>
          </div>
          <div class="tile" title={a.key == null ? "Not certain enough to say" : undefined}>
            <span class="label">Key</span>
            <span class="value">{a.key ?? "—"}</span>
          </div>
        </div>
        <p class="note">
          Estimated by Encore from the audio.
          {#if info?.canAnalyse}<button class="link" onclick={() => d.estimate()}>Estimate again from the {where}</button>{/if}
        </p>
      {:else if d.estimating}
        <div class="band">
          <Icon name="sparkle" size={18} />
          <div class="grow">
            <b>Listening to the {where ?? "file"}…</b>
            <div class="bar" role="progressbar" aria-label="Estimating"><i></i></div>
          </div>
          <button class="pill" onclick={() => d.cancel()}>Cancel</button>
        </div>
      {:else if info?.canAnalyse}
        <div class="band">
          <Icon name="sparkle" size={18} />
          <div class="grow">
            <b>Tempo and key</b>
            <p>Estimated from the {where} on this computer. Nothing is uploaded.</p>
          </div>
          <button class="pill primary" onclick={() => d.estimate()}>Estimate</button>
        </div>
      {:else if info}
        <div class="band quiet">
          <Icon name="sparkle" size={18} />
          <p>Play this song through or download it, and Encore can estimate its tempo and key.</p>
        </div>
      {:else}
        <div class="band quiet"><p>Loading…</p></div>
      {/if}
      {#if d.error}<p class="err" role="alert">{d.error}</p>{/if}
    </section>

    <dl>
      {#if t.album}
        <dt>Album</dt>
        <dd>
          {#if t.album.id}<button class="link" onclick={goAlbum}>{t.album.name}</button>{:else}{t.album.name}{/if}{#if d.year}<span class="dim">&nbsp;· {d.year}</span>{/if}
        </dd>
      {/if}
      {#if t.duration}
        <dt>Length</dt>
        <dd class="mono">{fmt(t.duration)}</dd>
      {/if}
      {#if plays}
        <dt>Plays</dt>
        <dd>{plays}</dd>
      {/if}
      {#if fileLine}
        <dt>File</dt>
        <dd>{fileLine}</dd>
      {/if}
    </dl>

    <div class="btns">
      <button class="pill" onclick={() => d.close()}>Close</button>
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
    width: min(460px, calc(100vw - 48px));
    padding: 22px 24px 20px;
    border-radius: 16px;
    background: var(--raise);
    box-shadow: var(--shadow);
    animation: pop 0.16s var(--ease);
    display: grid;
    gap: 18px;
  }
  .sheet:focus {
    outline: none;
  }
  header {
    display: flex;
    gap: 14px;
    align-items: center;
    min-width: 0;
  }
  .art {
    width: 60px;
    flex: none;
    border-radius: 8px;
    box-shadow: 0 8px 24px -10px rgb(0 0 0 / 0.5);
  }
  .who {
    min-width: 0;
  }
  h2 {
    font-family: var(--font-display);
    font-size: 21px;
    font-weight: 650;
    letter-spacing: -0.015em;
    line-height: 1.15;
  }
  .who p {
    margin-top: 3px;
    color: var(--mute);
  }
  .measure {
    display: grid;
    gap: 8px;
  }
  .tiles {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .tile {
    display: grid;
    gap: 2px;
    padding: 12px 14px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--accent) 9%, var(--panel));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .label {
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--mute);
  }
  .value {
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 650;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .value.mono {
    font-variant-numeric: tabular-nums;
  }
  .value small {
    margin-left: 5px;
    font-family: var(--font-ui);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0;
    color: var(--mute);
  }
  .band {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
    border-radius: 12px;
    background: var(--hover);
    color: var(--accent);
  }
  .band.quiet {
    color: var(--dim);
  }
  .band b {
    color: var(--text);
    font-weight: 600;
  }
  .band p {
    font-size: 13px;
    color: var(--mute);
  }
  .grow {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .bar {
    margin-top: 7px;
    height: 3px;
    border-radius: 2px;
    background: var(--line);
    overflow: hidden;
  }
  .bar i {
    display: block;
    width: 35%;
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    animation: slide 1.1s var(--ease) infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(290%);
    }
  }
  .note {
    font-size: 12.5px;
    color: var(--mute);
  }
  .link {
    color: var(--text);
    font-weight: 550;
  }
  .note .link {
    margin-left: 4px;
  }
  .link:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .err {
    font-size: 13px;
    color: hsl(0 70% 62%);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 9px 18px;
    margin: 0;
    padding-top: 14px;
    border-top: 1px solid var(--line);
    font-size: 13.5px;
  }
  dt {
    color: var(--mute);
  }
  dd {
    margin: 0;
    min-width: 0;
  }
  .dim {
    color: var(--mute);
  }
  .btns {
    display: flex;
    justify-content: flex-end;
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
