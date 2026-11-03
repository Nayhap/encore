<script lang="ts">
  import { onMount } from "svelte";
  import { call, isTauri, type AccountChannel, type BrowserProfile } from "$lib/backend";
  import { account, ui } from "$lib/state.svelte";
  import { os, secretStore } from "$lib/platform";
  import { settings } from "./shared.svelte";

  // What browser import can and can't do on this OS (D8).
  const importHelp =
    os === "macos"
      ? "Uses the YouTube session already signed in to your browser. For Edge, Chrome and Brave, macOS asks once to allow access to the browser's “Safe Storage” key in the Keychain; choose Always Allow."
      : os === "windows"
        ? "Uses the YouTube session already signed in to Firefox. Current Edge and Chrome lock their cookies while they run and protect them with app-bound encryption that other apps can't read, so importing from them usually fails. For those, use Sign in above."
        : "Uses the YouTube session already signed in to your browser. For Chrome, Chromium, Edge and Brave, Encore reads the browser's key from GNOME Keyring or KWallet, which may ask you to unlock it.";
  const browserNames: Record<string, string> = { edge: "Microsoft Edge", chrome: "Google Chrome", brave: "Brave", chromium: "Chromium", firefox: "Firefox", safari: "Safari", webview: "Encore sign-in" };
  const sourceName = (s: string) => browserNames[s.split(":")[0]] ?? s;

  let profiles = $state<BrowserProfile[]>([]);
  let profile = $state("");
  let pickKey = $state<string | null>(null);
  let switching = $state(false);
  let channels = $state<AccountChannel[]>([]);
  const busy = $derived(settings.busy);
  const run = <T,>(what: string, f: () => Promise<T>) => settings.run(what, f);

  onMount(async () => {
    profiles = await call((a) => a.browserProfiles()).catch(() => []);
    // Windows: Firefox is the browser import can actually read (D8).
    const prefer = os === "windows" ? "firefox:" : "edge:";
    profile = profiles.find((p) => p.id.startsWith(prefer))?.id ?? profiles[0]?.id ?? "";
  });

  const st = $derived(account.state);
  const shownChannels = $derived(st.state === "choosingChannel" ? st.channels : switching ? channels : []);
  $effect(() => {
    if (shownChannels.length && !shownChannels.some((c) => c.key === pickKey)) pickKey = shownChannels[0].key;
  });

  const importNow = () => run("import", async () => (account.state = await call((a) => a.importBrowser(profile))));
  const signIn = () => run("signin", () => call((a) => a.beginSignin()));
  async function choose() {
    if (!pickKey) return;
    const s = await run("select", () => call((a) => a.selectChannel(pickKey!)));
    if (s) {
      account.state = s;
      switching = false;
      ui.say(s.state === "signedIn" ? `Signed in as ${s.channel.name}` : "Channel saved");
    }
  }
  async function startSwitch() {
    const c = await run("channels", () => call((a) => a.reloadChannels()));
    if (c) {
      channels = c;
      switching = true;
    }
  }
  const signOut = () => run("signout", async () => {
    await call((a) => a.signOut());
    account.state = { state: "anonymous" };
    ui.say("Signed out");
  });
  const stageText: Record<string, string> = {
    opened: "Sign in in the window that opened. Encore waits until you are back on YouTube Music.",
    validating: "Checking the session…",
    chooseChannel: "Signed in. Choose the channel to use below.",
    rejected: "Google refused to sign in inside Encore. Use “Import from browser” instead.",
    cancelled: "Sign-in window closed.",
    failed: "Sign-in did not finish.",
    error: "The session could not be checked yet; finish signing in, or close the window.",
  };
</script>

<section>
  <h2>Account</h2>
  {#if st.state === "signedIn"}
    <div class="row">
      <div>
        <b>Signed in as {st.channel.name}</b>
        <p>{st.channel.isBrand ? "Brand channel" : "Personal channel"}{st.channel.handle ? ` · ${st.channel.handle}` : ""} · from {sourceName(st.source)}</p>
      </div>
      <div class="btns">
        <button class="pill" disabled={!!busy} onclick={startSwitch}>Switch channel</button>
        <button class="pill" disabled={!!busy} onclick={signOut}>Sign out</button>
      </div>
    </div>
  {:else if st.state === "expired"}
    <div class="row">
      <div>
        <b>Session expired</b>
        <p>{st.reason} Imported sessions stop working when the browser rotates its cookies. Import again, or sign in inside Encore.</p>
      </div>
      <div class="btns">
        <button class="pill primary" disabled={!!busy || !isTauri} onclick={signIn}>{busy === "signin" ? "Opening…" : "Sign in"}</button>
        <button class="pill" disabled={!!busy} onclick={signOut}>Sign out</button>
      </div>
    </div>
  {:else if st.state === "choosingChannel"}
    <div class="row">
      <div>
        <b>Choose a channel</b>
        <p>{st.channels.length ? "Pick the channel Encore should use below." : "YouTube Music didn't list a channel for this session; it's probably signed out. Sign out here, then sign in again or import from a browser where you're signed in."}</p>
      </div>
      <button class="pill" disabled={!!busy} onclick={signOut}>{st.channels.length ? "Cancel" : "Sign out"}</button>
    </div>
  {:else if st.state === "anonymous"}
    <div class="row">
      <div>
        <b>YouTube Music</b>
        <p>Sign in to see your library, likes and playlists. Sign-in waits until you have picked an account, including brand accounts.</p>
      </div>
      <button class="pill primary" disabled={!!busy || !isTauri} onclick={signIn}>{busy === "signin" ? "Opening…" : "Sign in"}</button>
    </div>
  {/if}

  {#if st.state !== "signedIn"}
    <div class="row">
      <div>
        <b>Import from browser</b>
        <p>{importHelp} Your session is stored only in {secretStore}.</p>
      </div>
      <div class="btns">
        <select bind:value={profile} aria-label="Browser profile" disabled={!profiles.length}>
          {#each profiles as p}<option value={p.id}>{p.browser} — {p.name}</option>{/each}
          {#if !profiles.length}<option value="">No browsers found</option>{/if}
        </select>
        <button class="pill primary" disabled={!profile || !!busy} onclick={importNow}>{busy === "import" ? "Importing…" : "Import"}</button>
      </div>
    </div>
  {/if}

  {#if account.signin && stageText[account.signin.stage] && st.state !== "signedIn"}
    <p class="note">{stageText[account.signin.stage]}{account.signin.message ? ` ${account.signin.message}` : ""}</p>
  {/if}

  {#if shownChannels.length}
    <div class="picker" role="radiogroup" aria-label="Choose a channel">
      <p class="note">Choose the channel Encore should use. Brand channels have their own library and history.</p>
      {#each shownChannels as c (c.key)}
        <label class="chan">
          <input type="radio" name="chan" value={c.key} bind:group={pickKey} />
          <span><b>{c.name}</b>{#if c.handle}<small> {c.handle}</small>{/if}{#if c.isBrand}<small> · brand</small>{/if}</span>
        </label>
      {/each}
      <div class="btns">
        <button class="pill primary" disabled={!pickKey || !!busy} onclick={choose}>{busy === "select" ? "Checking…" : "Continue"}</button>
        {#if switching}<button class="pill" onclick={() => (switching = false)}>Cancel</button>{/if}
      </div>
    </div>
  {/if}
  {#if settings.err}<p class="note error" role="alert">{settings.err}</p>{/if}
</section>
