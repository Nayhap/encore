#!/usr/bin/env bash
# Build latest.json for in-app updates (D24) from the files a release
# uploads, and refuse to if anything is off.
#
#   scripts/release-manifest.sh <tag> <dist>
#
# <dist> holds the release's files (the build jobs' artifacts). Writes
# <dist>/latest.json and fails unless:
# - the tag is the version in tauri.conf.json, both Cargo.toml and package.json;
# - there is exactly one updater package per platform, with its .sig:
#   the macOS universal .app.tar.gz (listed for darwin-aarch64 and
#   darwin-x86_64), the NSIS -setup.exe, the .msi and the AppImage;
# - every signature verifies with the public key in tauri.conf.json
#   (minisign -Vm) and was made for this version.
# The notes are the "What's new" and "Fixes" sections of
# .github/release-notes/<tag>.md. ENCORE_ROOT points at another checkout
# (the tests use it).
set -euo pipefail

tag=${1:?usage: release-manifest.sh <tag> <dist>}
dist=${2:?usage: release-manifest.sh <tag> <dist>}
root=${ENCORE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
repo=${ENCORE_REPO:-Nayhap/encore}
version=${tag#v}

fail() {
  echo "release-manifest: $*" >&2
  exit 1
}

command -v jq >/dev/null || fail "jq is needed"
command -v minisign >/dev/null || fail "minisign is needed"

# The same version everywhere.
conf="$root/src-tauri/tauri.conf.json"
for v in \
  "tauri.conf.json=$(jq -r .version "$conf")" \
  "package.json=$(jq -r .version "$root/package.json")" \
  "Cargo.toml=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)" \
  "src-tauri/Cargo.toml=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/src-tauri/Cargo.toml" | head -1)"; do
  [ "${v#*=}" = "$version" ] || fail "$tag doesn't match ${v%%=*} (${v#*=})"
done

# One file per platform, by the pattern the bundler names it with.
one() {
  local found
  found=$(find "$dist" -type f -name "$1" ! -name '*.sig' | sort)
  [ -n "$found" ] || fail "no $2 ($1) in $dist"
  [ "$(printf '%s\n' "$found" | wc -l | tr -d ' ')" = 1 ] || fail "more than one $2 in $dist: $(echo $found)"
  [ -f "$found.sig" ] || fail "$found has no .sig"
  printf '%s' "$found"
}
mac=$(one '*.app.tar.gz' 'macOS updater package')
nsis=$(one '*-setup.exe' 'NSIS installer')
msi=$(one '*.msi' 'MSI installer')
appimage=$(one '*.AppImage' 'AppImage')

# Every signature checks out with the key Encore ships, for this version.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
jq -r '.plugins.updater.pubkey' "$conf" | base64 -d > "$tmp/key.pub" || fail "no public key in tauri.conf.json"
for f in "$mac" "$nsis" "$msi" "$appimage"; do
  base64 -d < "$f.sig" > "$tmp/sig" || fail "$f.sig isn't base64"
  out=$(minisign -Vm "$f" -p "$tmp/key.pub" -x "$tmp/sig" 2>&1) || fail "$(basename "$f"): the signature doesn't verify with the public key in tauri.conf.json"
  signed=$(printf '%s\n' "$out" | sed -n 's/.*version:\([^[:space:]]*\).*/\1/p' | head -1)
  [ "$signed" = "$version" ] || fail "$(basename "$f") was signed for version '${signed:-none}', not $version"
done

# "What's new" and "Fixes", headings included.
notes_file="$root/.github/release-notes/$tag.md"
notes=""
if [ -f "$notes_file" ]; then
  # A section ends at the next heading, or at a paragraph after its list
  # (the closing line of the notes).
  notes=$(awk '
    /^## / { keep = ($0 ~ /^## (What.s new|Fixes)[[:space:]]*$/); listed = 0 }
    keep && !/^## / {
      if ($0 ~ /^- /) listed = 1
      else if (listed && blank && $0 != "" && $0 !~ /^[[:space:]]/) keep = 0
    }
    keep { print }
    { blank = ($0 == "") }
  ' "$notes_file" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}')
fi

url() { printf 'https://github.com/%s/releases/download/%s/%s' "$repo" "$tag" "$(basename "$1")"; }
sig() { tr -d '\n' < "$1.sig"; }
jq -n \
  --arg version "$version" \
  --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg notes "$notes" \
  --arg mac "$(url "$mac")" --arg mac_sig "$(sig "$mac")" \
  --arg nsis "$(url "$nsis")" --arg nsis_sig "$(sig "$nsis")" \
  --arg msi "$(url "$msi")" --arg msi_sig "$(sig "$msi")" \
  --arg appimage "$(url "$appimage")" --arg appimage_sig "$(sig "$appimage")" \
  '{
    version: $version,
    pub_date: $date,
    notes: $notes,
    platforms: {
      "darwin-aarch64": { url: $mac, signature: $mac_sig },
      "darwin-x86_64": { url: $mac, signature: $mac_sig },
      "windows-x86_64-nsis": { url: $nsis, signature: $nsis_sig },
      "windows-x86_64-msi": { url: $msi, signature: $msi_sig },
      "linux-x86_64": { url: $appimage, signature: $appimage_sig }
    }
  }' > "$dist/latest.json"
echo "release-manifest: wrote $dist/latest.json for $version"
