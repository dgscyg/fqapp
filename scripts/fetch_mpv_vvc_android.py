#!/usr/bin/env python3
"""Stage the prebuilt VVC-capable libmpv for the Android media_kit player.

The libraries are used as published, not rebuilt:

* libmpv + FFmpeg (with VVC and libvvdec) + their runtime come from the
  VVCEasy v4.0.0 build of mpv-android-vvc (MPV-VVCEasy-release-signed.apk).
  The APK's own JNI wrapper (libplayer.so) is not used.
* libmediakitandroidhelper.so (JavaVM/Surface bridge that package:media_kit
  loads) comes from media-kit/libmpv-android-video-build v1.1.7, the build
  media_kit_libs_android_video 1.3.8 pins.

Every download is pinned by SHA-256. Output: android/app/src/mpv/jniLibs/<abi>/
(git-ignored), which app/build.gradle.kts adds as a jniLibs source. Without it
the APK still builds; the app then reports VVC as unavailable and keeps H.265.

usage: python3 scripts/fetch_mpv_vvc_android.py [--cache DIR] [--out DIR]
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import sys
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ABIS = ("arm64-v8a", "armeabi-v7a")
VVCEASY_APK = (
    "https://github.com/MartinEesmaa/VVCEasy/releases/download/v4.0.0/MPV-VVCEasy-release-signed.apk",
    "e467d4fcb6a5d529121e2b91b3f3c7e0d1a525bdd0ce7748b214c86a5795dd5f",
)
MEDIA_KIT_JARS = {
    "arm64-v8a": (
        "https://github.com/media-kit/libmpv-android-video-build/releases/download/v1.1.7/default-arm64-v8a.jar",
        "4363dfa5d3d415b91c1f16f6fb90c3fe59a77dfd3f9b824d2b24b492d6b09df9",
    ),
    "armeabi-v7a": (
        "https://github.com/media-kit/libmpv-android-video-build/releases/download/v1.1.7/default-armeabi-v7a.jar",
        "8ead114fc5a43348d89dc0eb8f41823e549b15115c29f73ee26973f973620995",
    ),
}
# libmpv's full NEEDED closure inside the APK (system libraries excluded).
APK_LIBRARIES = (
    "libmpv.so",
    "libavcodec.so",
    "libavdevice.so",
    "libavfilter.so",
    "libavformat.so",
    "libavutil.so",
    "libswresample.so",
    "libswscale.so",
    "libvvdec.so",
    "libfdk-aac.so",
    "libopus.so",
    "libc++_shared.so",
)
HELPER = "libmediakitandroidhelper.so"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(url: str, expected: str, cache: Path) -> Path:
    target = cache / url.rsplit("/", 1)[1]
    if target.is_file() and sha256(target) == expected:
        return target
    cache.mkdir(parents=True, exist_ok=True)
    partial = target.with_suffix(target.suffix + ".download")
    for attempt in range(1, 4):
        try:
            print(f"Downloading {target.name} (attempt {attempt}/3)", flush=True)
            with urllib.request.urlopen(url, timeout=120) as response, partial.open("wb") as out:
                shutil.copyfileobj(response, out, 1 << 20)
            actual = sha256(partial)
            if actual != expected:
                raise RuntimeError(f"SHA-256 mismatch for {target.name}: {actual}")
            partial.replace(target)
            return target
        except Exception as error:  # retried, then re-raised
            print(f"  failed: {error}", file=sys.stderr)
            if attempt == 3:
                raise
        finally:
            partial.unlink(missing_ok=True)
    raise AssertionError("unreachable")


def extract(archive: Path, member: str, destination: Path) -> None:
    with zipfile.ZipFile(archive) as zf:
        with zf.open(member) as source, destination.open("wb") as out:
            shutil.copyfileobj(source, out, 1 << 20)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--cache", default=os.environ.get(
        "FQAPP_MPV_CACHE", str(Path.home() / ".cache/fqapp-mpv-vvc")))
    parser.add_argument("--out", default=str(ROOT / "android/app/src/mpv/jniLibs"))
    args = parser.parse_args()
    cache, out = Path(args.cache), Path(args.out)
    apk = fetch(*VVCEASY_APK, cache)
    staging = Path(tempfile.mkdtemp(prefix="mpv-vvc-", dir=out.parent if out.parent.exists() else None))
    try:
        for abi in ABIS:
            abi_dir = staging / abi
            abi_dir.mkdir(parents=True)
            for name in APK_LIBRARIES:
                extract(apk, f"lib/{abi}/{name}", abi_dir / name)
            jar = fetch(*MEDIA_KIT_JARS[abi], cache)
            extract(jar, f"lib/{abi}/{HELPER}", abi_dir / HELPER)
        if out.exists():
            shutil.rmtree(out)
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(str(staging), str(out))
    finally:
        shutil.rmtree(staging, ignore_errors=True)
    for abi in ABIS:
        total = sum(p.stat().st_size for p in (out / abi).iterdir())
        print(f"{abi}: {len(APK_LIBRARIES) + 1} libraries, {total / 1048576:.1f} MiB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
