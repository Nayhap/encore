#!/usr/bin/env bash
# Tests for release-manifest.sh (D24) on a made-up checkout and dist/,
# signed with a throwaway key. Needs jq and minisign. CI runs it on Linux.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

key="$work/key"
minisign -G -W -p "$key.pub" -s "$key.sec" -f >/dev/null

checkout() {
  local v=$1 root="$work/root"
  rm -rf "$root"
  mkdir -p "$root/src-tauri" "$root/.github/release-notes"
  jq -n --arg v "$v" --arg k "$(base64 < "$key.pub" | tr -d '\n')" '{version: $v, plugins: {updater: {pubkey: $k}}}' > "$root/src-tauri/tauri.conf.json"
  jq -n --arg v "$v" '{version: $v}' > "$root/package.json"
  printf '[workspace.package]\nversion = "%s"\n' "$v" > "$root/Cargo.toml"
  printf '[package]\nname = "encore"\nversion = "%s"\n' "$v" > "$root/src-tauri/Cargo.toml"
  cat > "$root/.github/release-notes/v$v.md" <<'EOF'
Intro that stays out.

## Download

| a | b |

## What's new

- **Updates** in the app.

## Fixes

- A fix.

Encore is unofficial.
EOF
}

# A signed file, the way the Tauri CLI signs: version in the trusted comment.
signed() {
  local f=$1 v=$2
  mkdir -p "$(dirname "$f")"
  head -c 2048 /dev/urandom > "$f"
  minisign -S -s "$key.sec" -m "$f" -x "$f.minisig" -t "timestamp:1700000000	file:$(basename "$f")	version:$v" >/dev/null
  base64 < "$f.minisig" | tr -d '\n' > "$f.sig"
  rm "$f.minisig"
}

dist_for() {
  local v=$1 d="$work/dist"
  rm -rf "$d"
  signed "$d/Encore_${v}_universal.app.tar.gz" "$v"
  signed "$d/Encore_${v}_x64-setup.exe" "$v"
  signed "$d/Encore_${v}_x64_en-US.msi" "$v"
  signed "$d/Encore_${v}_amd64.AppImage" "$v"
  : > "$d/Encore_${v}_universal.dmg"
  : > "$d/Encore_${v}_amd64.deb"
}

run() { ENCORE_ROOT="$work/root" "$here/release-manifest.sh" "$@" 2>&1; }
pass=0
ok() { pass=$((pass + 1)); echo "ok - $1"; }
expect_fail() {
  local what=$1 needle=$2
  shift 2
  local out
  if out=$(run "$@"); then
    echo "not ok - $what: succeeded"
    exit 1
  fi
  case "$out" in *"$needle"*) ok "$what" ;; *) echo "not ok - $what: $out"; exit 1 ;; esac
}

# The good case.
checkout 0.9.0
dist_for 0.9.0
run v0.9.0 "$work/dist" >/dev/null
m="$work/dist/latest.json"
[ "$(jq -r .version "$m")" = 0.9.0 ] && ok "version"
[ "$(jq -r '.platforms | keys | join(",")' "$m")" = "darwin-aarch64,darwin-x86_64,linux-x86_64,windows-x86_64-msi,windows-x86_64-nsis" ] && ok "five platforms"
[ "$(jq -r '.platforms["darwin-aarch64"].url' "$m")" = "https://github.com/Nayhap/encore/releases/download/v0.9.0/Encore_0.9.0_universal.app.tar.gz" ] && ok "urls from the real file names"
[ "$(jq -r '.platforms["darwin-x86_64"].signature' "$m")" = "$(cat "$work/dist/Encore_0.9.0_universal.app.tar.gz.sig")" ] && ok "whole .sig contents"
notes=$(jq -r .notes "$m")
case "$notes" in "## What's new"*"Updates"*"## Fixes"*"A fix."*) ok "notes" ;; *) echo "not ok - notes: $notes"; exit 1 ;; esac
case "$notes" in *Download*|*unofficial*|*Intro*) echo "not ok - other sections leaked: $notes"; exit 1 ;; esac
jq -e '.pub_date | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T")' "$m" >/dev/null && ok "date"

# What must stop a release.
expect_fail "tag vs versions" "doesn't match" v0.9.1 "$work/dist"
rm "$work/dist/Encore_0.9.0_x64_en-US.msi"*
expect_fail "missing platform" "no MSI installer" v0.9.0 "$work/dist"
dist_for 0.9.0
signed "$work/dist/old/Encore_0.8.0_amd64.AppImage" 0.9.0
expect_fail "duplicate" "more than one AppImage" v0.9.0 "$work/dist"
dist_for 0.9.0
rm "$work/dist/Encore_0.9.0_x64-setup.exe.sig"
expect_fail "missing .sig" "has no .sig" v0.9.0 "$work/dist"
dist_for 0.9.0
printf 'x' >> "$work/dist/Encore_0.9.0_x64-setup.exe"
expect_fail "tampered file" "doesn't verify" v0.9.0 "$work/dist"
dist_for 0.9.0
signed "$work/dist/Encore_0.9.0_universal.app.tar.gz" 0.8.0
expect_fail "signed for another version" "signed for version '0.8.0'" v0.9.0 "$work/dist"
dist_for 0.9.0
minisign -G -W -p "$work/other.pub" -s "$work/other.sec" -f >/dev/null
f="$work/dist/Encore_0.9.0_amd64.AppImage"
minisign -S -s "$work/other.sec" -m "$f" -x "$f.minisig" -t "version:0.9.0" >/dev/null
base64 < "$f.minisig" | tr -d '\n' > "$f.sig"
expect_fail "another key" "doesn't verify" v0.9.0 "$work/dist"

echo "release-manifest: $pass checks passed"
