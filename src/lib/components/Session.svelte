<script lang="ts">
  // The player bar's one session control: the sleep timer and the private
  // session (D26, D36). Idle it is a moon; while either runs it says so in
  // a few words ("23 min · Private"), and it opens one popover for both.
  import { SLEEP_CHOICES, account, player, sleepTimer, taste } from "$lib/state.svelte";
  import Icon from "./Icon.svelte";

  let open = $state(false);
  let wrap = $state<HTMLDivElement>();
  let pop = $state<HTMLDivElement>();
  let btn = $state<HTMLButtonElement>();

  const sleep = $derived(player.sleep);
  const priv = $derived(taste.private);
  // Signed out nothing reaches a history, so there's no private session to offer.
  const canPrivate = $derived(account.signedIn || priv);
  const label = $derived([sleepTimer.short, priv ? "Private" : null].filter(Boolean).join(" · "));
  const title = $derived(
    [sleep ? sleepTimer.status : null, priv ? `Private session${taste.privateLeftText ? `, ${taste.privateLeftText}` : ""}` : null].filter(Boolean).join(". ") ||
      (canPrivate ? "Sleep timer and private session" : "Sleep timer"),
  );

  $effect(() => {
    if (!open || !pop) return;
    pop.querySelector<HTMLElement>("button, input")?.focus();
    const away = (e: MouseEvent) => !wrap?.contains(e.target as Node) && (open = false);
    addEventListener("mousedown", away, true);
    return () => removeEventListener("mousedown", away, true);
  });

  function key(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    open = false;
    btn?.focus();
  }

  const isTimer = (m: number) => sleep?.mode === "timer" && sleepTimer.chosen === m;
  const looping = $derived(!!player.abLoop);
</script>

<div class="session" bind:this={wrap}>
  <button
    bind:this={btn}
    class={label ? "chip" : "icon-btn"}
    class:open
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label={label ? `${title}. Change` : title}
    {title}
    onclick={() => (open = !open)}
  >
    <Icon name={priv && !sleep ? "lock" : "moon"} size={label ? 14 : 18} />
    {#if label}<span class="num">{label}</span>{/if}
  </button>

  {#if open}
    <div class="pop" role="dialog" aria-label="Sleep timer and private session" tabindex="-1" bind:this={pop} onkeydown={key}>
      <section>
        <header>
          <h4>Sleep timer</h4>
          {#if sleep}<button class="link" onclick={() => sleepTimer.off()}>Turn off</button>{/if}
        </header>
        <p class="line">{sleep ? sleepTimer.status : "Pause the music after a while."}</p>
        <div class="choices">
          {#each SLEEP_CHOICES as [m, text] (m)}
            <button class="choice" class:on={isTimer(m)} aria-pressed={isTimer(m)} onclick={() => sleepTimer.set("timer", m)}>{text}</button>
          {/each}
          <button
            class="choice wide"
            class:on={sleep?.mode === "endOfSong"}
            aria-pressed={sleep?.mode === "endOfSong"}
            disabled={looping && sleep?.mode !== "endOfSong"}
            title={looping ? "Clear the loop first: a looping song doesn't end" : undefined}
            onclick={() => sleepTimer.set("endOfSong")}
          >End of song</button>
          <button class="choice wide" class:on={sleep?.mode === "endOfQueue"} aria-pressed={sleep?.mode === "endOfQueue"} onclick={() => sleepTimer.set("endOfQueue")}>End of queue</button>
        </div>
      </section>

      {#if canPrivate}
        <section class="private">
          <label class="row">
            <span>
              <strong class="h">Private session</strong>
              <span class="line">
                {#if priv}
                  {taste.privateLeftText ? `Ends in ${taste.privateLeftText.replace(" left", "")}.` : "On until Encore quits."}
                {:else}
                  For an hour.
                {/if}
                Plays stay out of your YouTube history. YouTube still sees what you play, and Encore's stats still count it.
              </span>
            </span>
            <input type="checkbox" role="switch" checked={priv} onchange={(e) => void taste.setPrivate(e.currentTarget.checked)} />
          </label>
        </section>
      {/if}
    </div>
  {/if}
</div>

<style>
  .session {
    position: relative;
    display: flex;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px 0 9px;
    border-radius: 99px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    border: 1px solid var(--line);
    white-space: nowrap;
    transition: background-color 0.15s, border-color 0.15s;
  }
  .chip :global(svg) {
    color: var(--accent);
  }
  .chip:hover,
  .chip.open {
    background: var(--hover);
    border-color: color-mix(in srgb, var(--accent) 40%, var(--line));
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  /* A narrow bar keeps the words and drops the icon. */
  @media (max-width: 1000px) {
    .chip :global(svg) {
      display: none;
    }
    .chip {
      padding: 0 10px;
    }
  }
  .icon-btn.open {
    color: var(--text);
  }

  .pop {
    position: absolute;
    right: -8px;
    bottom: calc(100% + 14px);
    z-index: 40;
    width: 300px;
    padding: 6px;
    border-radius: 14px;
    background: var(--raise);
    box-shadow: var(--shadow);
    animation: pop 0.14s var(--ease);
    transform-origin: bottom right;
  }
  section {
    padding: 10px 10px 12px;
  }
  section + section {
    border-top: 1px solid var(--line);
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }
  h4,
  .h {
    display: block;
    margin: 0;
    font-size: 13.5px;
    font-weight: 650;
  }
  .line {
    display: block;
    margin: 3px 0 0;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--mute);
    text-wrap: pretty;
  }
  .link {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--accent);
  }
  .link:hover {
    text-decoration: underline;
  }
  .choices {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
    margin-top: 12px;
  }
  .choice {
    height: 30px;
    border-radius: 8px;
    font-size: 12.5px;
    font-weight: 550;
    color: var(--text);
    background: color-mix(in srgb, var(--text) 6%, transparent);
    transition: background-color 0.15s;
  }
  .choice.wide {
    grid-column: span 2;
  }
  .choice:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 11%, transparent);
  }
  .choice.on {
    color: var(--accent-ink);
    background: var(--accent);
  }
  .choice:disabled {
    color: var(--dim);
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    cursor: pointer;
  }
  .row > span {
    flex: 1;
  }
  .row input {
    margin-top: 1px;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(4px) scale(0.98);
    }
  }
</style>
