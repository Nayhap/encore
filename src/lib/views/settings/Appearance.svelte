<script lang="ts">
  import { isTauri } from "$lib/backend";
  import { commands } from "$lib/backend/bindings";
  import { isMac, os } from "$lib/platform";
  import { ambience, ui, type Theme } from "$lib/state.svelte";
  import Icon from "$lib/components/Icon.svelte";

  const themes: { v: Theme; label: string; icon: string }[] = [
    { v: "system", label: "Match system", icon: "settings" },
    { v: "dark", label: "Dark", icon: "moon" },
    { v: "light", label: "Light", icon: "sun" },
  ];

  function setTitlebar(on: boolean) {
    ui.systemTitlebar = on;
    localStorage.setItem("titlebar", on ? "system" : "encore");
    if (isTauri) commands.setSystemTitlebar(on, true).catch((e: any) => ui.say(`Could not change the title bar: ${e?.message ?? e}`));
  }
</script>

<section>
  <h2>Appearance</h2>
  <div class="row">
    <div><b>Theme</b></div>
    <div class="seg" role="radiogroup" aria-label="Theme">
      {#each themes as t}
        <button role="radio" aria-checked={ui.theme === t.v} onclick={() => (ui.theme = t.v)}>
          <Icon name={t.icon} size={15} />{t.label}
        </button>
      {/each}
    </div>
  </div>
  <label class="row">
    <div>
      <b>Colour from artwork</b>
      <p>Tint highlights and backgrounds with colours from the song that is playing.</p>
    </div>
    <input type="checkbox" role="switch" bind:checked={ui.tint} />
  </label>
  <!-- Lane E: the cover ambience (D27), under the colour it needs. Linux has no blur (D17), so no row. -->
  {#if os !== "linux"}
    <label class="row">
      <div>
        <b>Cover ambience</b>
        <p>{ui.tint ? "A soft, blurred wash of the cover behind now playing and full-screen lyrics." : "Turn on colour from artwork to use the cover's ambience."}</p>
      </div>
      <input type="checkbox" role="switch" checked={ambience.enabled && ui.tint} disabled={!ui.tint} style:opacity={ui.tint ? null : 0.45} onchange={(e) => ambience.set(e.currentTarget.checked)} />
    </label>
  {/if}
  {#if !isMac}
    <label class="row">
      <div>
        <b>Use the system title bar</b>
        <p>Show your desktop's own title bar and window buttons instead of Encore's.{os === "linux" ? " Encore restarts to apply it." : ""}</p>
      </div>
      <input type="checkbox" role="switch" checked={ui.systemTitlebar} onchange={(e) => setTitlebar(e.currentTarget.checked)} />
    </label>
  {/if}
</section>
