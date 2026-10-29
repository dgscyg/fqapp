#!/usr/bin/env bash
# Downloads the prebuilt libfqsign artifacts (obfuscated emulated signer)
# from the private dgscyg/rustnidbg release into rust/vendor/, where the
# Android/Windows build scripts pick them up automatically:
#
#   Android:  scripts/build_rust_backend.sh  -> jniLibs/<abi>/libfqsign.so
#   Windows:  flutter build windows          -> fqsign.dll next to fqapp.exe
#   Linux:    copy rust/vendor/libfqsign-linux-x86_64.so next to the
#             executable as libfqsign.so (or anywhere on the EmuSigner
#             search path: exe dir, ../lib, cwd, FQSIGN_LIB).
#
# Requires `gh` authenticated with access to dgscyg/rustnidbg
# (gh auth login / gh auth refresh -s workflow as needed).
#
# Usage: scripts/fetch_fqsign_artifacts.sh [tag]   (default v0.2.0)
set -euo pipefail

TAG="${1:-v0.2.0}"
REPO="dgscyg/rustnidbg"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/rust/vendor"

command -v gh >/dev/null 2>&1 || {
  echo "错误：需要 gh CLI（gh auth login 授权 dgscyg 账号）。" >&2
  exit 1
}
gh auth status >/dev/null 2>&1 || {
  echo "错误：gh 未登录（gh auth login）。" >&2
  exit 1
}

mkdir -p "$DIR"
echo "==> 下载 $REPO release $TAG -> $DIR"
gh release download "$TAG" --repo "$REPO" --dir "$DIR" --clobber \
  --pattern 'libfqsign-android-aarch64.so' \
  --pattern 'libfqsign-android-armv7.so' \
  --pattern 'libfqsign-linux-x86_64.so' \
  --pattern 'libfqsign-linux-aarch64.so' \
  --pattern 'libfqsign-linux-armv7.so' \
  --pattern 'fqsign-windows-x64.dll' \
  --pattern 'fqsign.h'

echo "==> 完成："
ls -lh "$DIR"
echo
echo "构建入口："
echo "  Android: ./scripts/build_rust_backend.sh"
echo "  Windows: flutter build windows --release"
echo "  Linux:   cp $DIR/libfqsign-linux-x86_64.so <exe目录>/libfqsign.so"