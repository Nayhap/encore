#!/usr/bin/env bash
# Runs `encore --self-test` from a built bundle, the way users run it.
#   scripts/self-test-bundle.sh appimage <file.AppImage> [video]
#   scripts/self-test-bundle.sh appimage-in <docker image> <file.AppImage> [video]
#   scripts/self-test-bundle.sh deb <file.deb> [video]
#   scripts/self-test-bundle.sh app <Encore.app> [video]
# A video id makes it resolve and decode a real stream too. YouTube's bot
# checks don't fail the test (CI runners are often refused); add --strict
# to the environment variable SELF_TEST_ARGS to make them count. The report
# must also name the install kind the bundle updates as (D24): appImage,
# deb or app. That marker is patched into the binary by the bundler and is
# all that routes an update to the right installer.
set -euo pipefail
kind=$1; shift
video_default=4D7u5KF7SP8
report=$(mktemp)
case "$kind" in
  appimage|appimage-in) want=appImage ;;
  deb) want=deb ;;
  app) want=app ;;
  *) want=unknown ;;
esac
# Run the self-test, show its report, keep it for the install kind check.
run() {
  local rc=0
  "$@" > "$report" || rc=$?
  cat "$report"
  [ "$rc" = 0 ] || exit "$rc"
}
case "$kind" in
  appimage)
    f=$(realpath "$1"); v=${2:-$video_default}
    work=$(mktemp -d); cd "$work"
    # No FUSE needed: unpack it and run its AppRun, like the runtime does.
    "$f" --appimage-extract >/dev/null
    run ./squashfs-root/AppRun --self-test "$v" ${SELF_TEST_ARGS:-}
    ;;
  appimage-in)
    img=$1; f=$(realpath "$2"); v=${3:-$video_default}
    # A fresh system with what README asks for and a desktop's basic
    # libraries (GTK), but no audio server and no development packages.
    run docker run --rm -v "$f:/encore.AppImage:ro" "$img" bash -euo pipefail -c "
      export DEBIAN_FRONTEND=noninteractive
      apt-get update -qq
      common='ca-certificates python3 python3-venv nodejs libegl1 libgl1 libgles2 libgbm1'
      apt-get install -y -qq --no-install-recommends \$common libgtk-3-0t64 libasound2t64 >/dev/null 2>&1 || \
      apt-get install -y -qq --no-install-recommends \$common libgtk-3-0 libasound2 >/dev/null
      # yt-dlp needs Python 3.11; Ubuntu 22.04's python3 is 3.10.
      python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' || apt-get install -y -qq python3.11 python3.11-venv >/dev/null
      useradd -m qa && cd /home/qa
      su qa -c 'cp /encore.AppImage ./e.AppImage && ./e.AppImage --appimage-extract >/dev/null && ./squashfs-root/AppRun --self-test $v ${SELF_TEST_ARGS:-}'
    "
    ;;
  deb)
    f=$(realpath "$1"); v=${2:-$video_default}
    sudo apt-get install -y "$f"
    run encore --self-test "$v" ${SELF_TEST_ARGS:-}
    ;;
  app)
    a=$1; v=${2:-$video_default}
    run "$a/Contents/MacOS/encore" --self-test "$v" ${SELF_TEST_ARGS:-}
    ;;
  *) echo "unknown kind $kind" >&2; exit 2 ;;
esac
got=$(sed -n '/^{/,$p' "$report" | jq -r '.update.install')
[ "$got" = "$want" ] || { echo "self-test: this bundle updates as '$got', expected '$want'" >&2; exit 1; }
echo "self-test: updates as $got"
rm -f "$report"
