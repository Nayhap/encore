<script lang="ts">
  import { setup } from "$lib/state.svelte";
  import { os } from "$lib/platform";
  import Icon from "./Icon.svelte";

  // The player components setup (D22): what Encore needs, how big it is and
  // where it goes; then real progress from Rust; errors with Try again.

  const st = $derived(setup.status);
  const p = $derived(setup.progress);
  const phase = $derived(p?.phase ?? null);
  const view = $derived(
    setup.installing ? "progress" : phase === "done" && st?.usable ? "done" : phase === "failed" || setup.error ? "error" : "offer",
  );

  // Same unit as Settings' cache sizes (and what file managers show).
  const mb = (b: number | null | undefined, digits = 0) => `${((b ?? 0) / 1048576).toFixed(digits)} MB`;
  const pct = (f: number) => `${Math.floor(Math.max(0, Math.min(1, f)) * 100)}%`;

  /** The folder, with the home folder shortened. */
  function shortPath(path: string) {
    if (os === "windows") return path.replace(/^[A-Za-z]:\\Users\\[^\\]+\\AppData\\Local/i, "%LOCALAPPDATA%");
    return path.replace(/^\/(Users|home)\/[^/]+/, "~");
  }

  const names = $derived((st?.components ?? []).map((c) => c.name));
  const list = $derived(names.length === 3 ? `${names[0]}, ${names[1]} and ${names[2]}` : names.join(", "));
  const update = $derived(!!p?.update);

  type Step = { key: string; label: string; detail: string; state: "done" | "active" | "todo" };
  const order = ["downloading", "verifying", "unpacking", "settingUp"] as const;
  const steps = $derived.by<Step[]>(() => {
    const at = phase === "waiting" ? -1 : order.indexOf(phase as (typeof order)[number]);
    const s = (i: number): Step["state"] => (i < at ? "done" : i === at ? "active" : "todo");
    const dl = p ? `${mb(p.downloaded, 1)} of ${mb(p.downloadTotal, 1)}` : "";
    const unpackPct = p && (p.unpackTotal ?? 0) > 0 ? pct((p.unpacked ?? 0) / (p.unpackTotal ?? 1)) : "";
    return [
      { key: "downloading", label: "Download", detail: at > 0 ? mb(p?.downloadTotal, 1) : at === 0 ? `${p?.component ? `${p.component} · ` : ""}${dl} · ${pct((p?.downloaded ?? 0) / Math.max(1, p?.downloadTotal ?? 1))}` : mb(p?.downloadTotal || st?.downloadBytes, 1), state: s(0) },
      { key: "verifying", label: "Verify", detail: "Each file is checked against its published checksum", state: s(1) },
      { key: "unpacking", label: "Unpack", detail: at === 2 ? `${p?.component ?? ""}${unpackPct ? ` · ${unpackPct}` : ""}` : "Into Encore's own folder", state: s(2) },
      { key: "settingUp", label: "Set up", detail: "Starting the player for the first time", state: s(3) },
    ];
  });

  let box = $state<HTMLDivElement>();
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    view;
    // The main action of each view takes focus.
    queueMicrotask(() => box?.querySelector<HTMLElement>("[data-autofocus]")?.focus());
  });
  $effect(() => () => opener?.isConnected && opener.focus());

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setup.hide();
    } else if (e.key === "Tab" && box) {
      const f = [...box.querySelectorAll<HTMLElement>("button:not(:disabled)")];
      const i = f.indexOf(document.activeElement as HTMLElement);
      if (e.shiftKey && i <= 0) (f.at(-1)?.focus(), e.preventDefault());
      else if (!e.shiftKey && i === f.length - 1) (f[0]?.focus(), e.preventDefault());
    }
  }
</script>

<div class="scrim" role="presentation" onmousedown={(e) => e.target === e.currentTarget && setup.hide()}>
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="setup-title" aria-describedby="setup-lead" tabindex="-1" bind:this={box} onkeydown={key}>
    {#if view === "offer"}
      <div class="badge" aria-hidden="true"><Icon name="download" size={22} /></div>
      <h2 id="setup-title">Set up playback</h2>
      <p id="setup-lead" class="lead">
        To play music, Encore needs three free, open-source components: {list}. It downloads them once into its own folder and changes nothing else on your computer.
      </p>
      <dl class="facts">
        <div><dt>Download</dt><dd>{mb(st?.downloadBytes)}</dd></div>
        <div><dt>On disk</dt><dd>about {mb(st?.diskBytes)}</dd></div>
        <div class="where"><dt>Goes to</dt><dd title={st?.location}>{#each shortPath(st?.location ?? "").split(/(?<=[\/\\])/) as part}{part}<wbr />{/each}</dd></div>
      </dl>
      <p class="note">
        {#if phase === "cancelled"}Setup was cancelled. {/if}You can keep browsing while it installs. When Encore updates, it updates these too, without asking again.
      </p>
      <div class="btns">
        <button class="pill" onclick={() => setup.hide()}>Not now</button>
        <button class="pill primary" data-autofocus onclick={() => setup.install()}>Install</button>
      </div>
    {:else if view === "progress"}
      <h2 id="setup-title">{update ? "Updating player components" : "Setting up playback"}</h2>
      <p id="setup-lead" class="lead">
        {#if phase === "waiting"}Another Encore window is setting up. This one carries on when it's done.
        {:else}This takes a minute or so. You can keep browsing.{/if}
      </p>
      <div class="bar" role="progressbar" aria-label="Setup progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.floor((p?.fraction ?? 0) * 100)}>
        <i style:width={pct(p?.fraction ?? 0)}></i>
      </div>
      <ol class="steps">
        {#each steps as s (s.key)}
          <li class={s.state} aria-current={s.state === "active" ? "step" : undefined}>
            <span class="dot" aria-hidden="true">{#if s.state === "done"}<Icon name="check" size={13} />{/if}</span>
            <span class="txt"><b>{s.label}</b><small class:num={s.key === "downloading" || s.key === "unpacking"}>{s.detail}</small></span>
          </li>
        {/each}
      </ol>
      <div class="btns">
        <button class="pill" onclick={() => setup.cancel()}>Cancel</button>
        <button class="pill primary" data-autofocus onclick={() => setup.hide()}>Keep browsing</button>
      </div>
    {:else if view === "error"}
      <div class="badge warn" aria-hidden="true"><Icon name="x" size={20} /></div>
      <h2 id="setup-title">Setup didn't finish</h2>
      <p id="setup-lead" class="lead">{setup.error ?? p?.error?.message ?? "Something went wrong."}</p>
      <p class="note">Nothing half-installed was kept. Encore picks up where it can.</p>
      <div class="btns">
        <button class="pill" onclick={() => setup.hide()}>Not now</button>
        <button class="pill primary" data-autofocus onclick={() => setup.install()}>Try again</button>
      </div>
    {:else}
      <div class="badge ok" aria-hidden="true"><Icon name="check" size={22} /></div>
      <h2 id="setup-title">Ready to play</h2>
      <p id="setup-lead" class="lead">Playback is set up. Encore keeps the components up to date by itself.</p>
      <div class="btns">
        <button class="pill primary" data-autofocus onclick={() => setup.hide()}>Done</button>
      </div>
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
    background: rgb(0 0 0 / 0.42);
    animation: fade 0.15s var(--ease);
  }
  .sheet {
    width: min(460px, calc(100vw - 48px));
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
  .badge.ok {
    background: color-mix(in srgb, hsl(145 55% 50%) 18%, transparent);
    color: hsl(145 50% 58%);
  }
  :global([data-theme="light"]) .badge.ok {
    color: hsl(145 55% 32%);
  }
  .badge.warn {
    background: color-mix(in srgb, hsl(0 70% 60%) 16%, transparent);
    color: hsl(0 70% 64%);
  }
  :global([data-theme="light"]) .badge.warn {
    color: hsl(0 60% 45%);
  }
  h2 {
    font-family: var(--font-display);
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .lead {
    margin-top: 6px;
    font-size: 13.5px;
    line-height: 1.5;
    color: var(--mute);
  }
  .facts {
    display: grid;
    margin: 16px 0 0;
    padding: 4px 14px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: var(--panel);
  }
  .facts div {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 0;
    font-size: 13px;
  }
  .facts div + div {
    border-top: 1px solid var(--line);
  }
  dt {
    color: var(--mute);
  }
  dd {
    margin: 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .facts div.where {
    display: grid;
    gap: 2px;
  }
  .where dd {
    font-family: var(--font-mono);
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text);
    overflow-wrap: anywhere;
    -webkit-user-select: text;
    user-select: text;
  }
  .note {
    margin-top: 12px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--mute);
  }
  .bar {
    position: relative;
    height: 6px;
    margin-top: 18px;
    border-radius: 99px;
    background: var(--hover);
    overflow: hidden;
  }
  .bar i {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: inherit;
    background: var(--accent);
    transition: width 0.2s linear;
  }
  .steps {
    display: grid;
    gap: 2px;
    margin: 16px 0 0;
    padding: 0;
    list-style: none;
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 0;
  }
  .dot {
    flex: none;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 1.5px solid var(--line);
  }
  .done .dot {
    border-color: transparent;
    background: var(--accent);
    color: var(--accent-ink);
  }
  .active .dot {
    border-color: var(--accent);
    border-right-color: transparent;
    animation: spin 0.9s linear infinite;
  }
  .txt {
    display: grid;
    min-width: 0;
  }
  .txt b {
    font-size: 13.5px;
    font-weight: 600;
  }
  .todo .txt b {
    color: var(--mute);
  }
  .txt small {
    font-size: 12px;
    color: var(--mute);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .todo .txt small {
    color: var(--dim);
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 20px;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
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
  @media (prefers-reduced-motion: reduce) {
    .active .dot {
      animation: none;
      border-right-color: var(--accent);
    }
  }
</style>
