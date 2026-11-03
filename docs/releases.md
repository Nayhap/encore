# Releases

Encore updates itself from the latest published GitHub release (D24 in
[decisions.md](decisions.md)). **Publishing a draft ships it to every
Encore within 6 hours**, and an installed update can't be taken back from
the app. Check before you publish.

## Making one

1. Bump the version in `src-tauri/tauri.conf.json`, `Cargo.toml`,
   `src-tauri/Cargo.toml` and `package.json` (the release fails if they
   differ from the tag).
2. Write `.github/release-notes/v<version>.md`. The **What's new** and
   **Fixes** sections are what Encore shows in its update sheet, as plain
   bullets: keep each bullet one sentence or two.
3. Push the tag (`v0.5.0`). `release.yml` builds and signs every platform,
   self-tests each bundle, writes `latest.json` and attaches everything to a
   draft. A tag with a dash (`v0.5.0-rc.1`) becomes a pre-release, which
   the updater never offers: use one to try a release from GitHub by
   pointing a test build's `update-endpoint` at its
   `releases/download/<tag>/latest.json`.

## Before publishing

- The build jobs and their self-tests are green on all three systems.
- The draft has `latest.json`, the `.dmg`, `.app.tar.gz`, `-setup.exe`,
  `.msi`, `.deb`, `.AppImage` and a `.sig` for each updater package. (The
  `release` job fails without them, and if a signature doesn't verify.)
- Install the new macOS, Windows and Linux builds over the previous release
  once, by hand, and play a song.
- The notes read well, and say what users have to do (a Keychain prompt,
  Smart App Control, a moved setting).
- Anything that changes stored data (the database, settings, downloads)
  also opens what the previous version wrote: users can't go back.

Then publish, and make sure it is marked **Latest**: that is the release
`releases/latest/download/latest.json` points at.

## Pulling a bad release

The fix is a new release with a higher version, as soon as you can:
Encores that installed the bad one stay on it (there are no downgrades),
and the new one replaces it everywhere, including where it is downloaded
but not installed yet.

To stop it spreading until then, mark it a pre-release (or delete it), and
`releases/latest` falls back to the release before. What that does depends
on whether the one before has a `latest.json`:

- **It has one** (a version that updates itself, older than the bad one):
  Encore stops offering the bad version at its next check, and an Encore
  that downloaded it but hasn't installed it drops it then.
- **It has none** (the bad release is the first that updates itself): the
  check finds no manifest, which Encore treats as a failed check, so an
  Encore that already downloaded the bad version **still installs it** when
  it quits. Attach to the release before a `latest.json` naming that
  release's own version, `{"version": "0.4.0", "platforms": {}}`: Encore
  then finds nothing newer than itself and drops the staged copy. (It is
  never installed, so it needs no signatures.)

Either way, a staged copy on someone's computer is only dropped by a check
that answers with an older or no newer version.

## The signing key

Updater packages are signed with a minisign key made with
`bun run tauri signer generate`. Its public half is
`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`; every Encore ever
shipped trusts only that key.

- The private key and its password are the Actions secrets
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
  given to the release build step and nothing else, and an offline copy is
  kept by the maintainer (on the release Mac: `~/.tauri/encore-updater.key`,
  password in the login Keychain), backed up somewhere that isn't GitHub.
- **Lost key:** no installed Encore can update again until users install a
  release with a new key by hand. Don't lose it.
- **Leaked key:** whoever has it can sign packages every Encore accepts, as
  long as they can also get a manifest in front of it. Make a new key,
  ship a release signed with the old key that carries the new public key,
  then retire the old one and change the secrets.
- Never commit either half of a private key, and never print the secrets
  in a workflow.
