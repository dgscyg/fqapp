import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// VVC 播放失败、自动退回 H.265 时的提示。
const vvcFallbackToast = 'VVC 播放失败，已切换为 H.265';

/// 短剧「视频编码」：H.265（默认）或原始 VVC（bytevc2）。
///
/// bytevc2 就是标准 VVC/H.266，只是 MP4 外壳是厂商私有的 `bvc2`/`bv2C`。
/// 选 VVC 时取流带 `codec=vvc`（Rust 保留 `codec_type=4`），播放走 libmpv：
/// Rust 本地代理边解密边把外壳改写成 `vvc1`/`vvcC`。VVC 只有软解，更耗电，
/// 所以默认仍是 H.265。
enum DramaVideoCodec {
  h265('H.265（默认）'),
  vvc('原始 VVC');

  const DramaVideoCodec(this.label);

  final String label;
}

class DramaCodecPreference {
  DramaCodecPreference._();

  static const _key = 'drama_video_codec';

  /// 进程内的当前值：起播路径不能等读盘（同 `DecoderPreferenceCache`）。
  static DramaVideoCodec current = DramaVideoCodec.h265;

  /// 有 VVC 版 libmpv 的平台：Android（预编译 mpv-android-vvc）和
  /// Windows（VVCEasy libmpv-2.dll）。Linux 用系统 libmpv，iOS/macOS 没有
  /// 视频播放器，都不提供这个选项。
  static bool get platformSupported =>
      !kIsWeb &&
      (defaultTargetPlatform == TargetPlatform.android ||
          defaultTargetPlatform == TargetPlatform.windows);

  /// 本次起播是否按 VVC 取流/播放。
  static bool get vvcActive =>
      platformSupported && current == DramaVideoCodec.vvc;

  static Future<void> load() async {
    try {
      final preferences = await SharedPreferences.getInstance();
      final saved = preferences.getString(_key);
      current = DramaVideoCodec.values.firstWhere(
        (value) => value.name == saved,
        orElse: () => DramaVideoCodec.h265,
      );
    } catch (_) {
      current = DramaVideoCodec.h265;
    }
  }

  /// 先落盘再改内存：写失败时界面与播放器保持一致。
  static Future<void> set(DramaVideoCodec value) async {
    final preferences = await SharedPreferences.getInstance();
    if (!await preferences.setString(_key, value.name)) {
      throw StateError('Could not save the drama video codec');
    }
    current = value;
  }

  @visibleForTesting
  static void resetForTest() => current = DramaVideoCodec.h265;
}

/// mpv 播放普通（H.264/H.265）流失败、自动退回 ExoPlayer 时的提示。
const mpvFallbackToast = 'mpv 播放失败，已切换为 ExoPlayer';

/// 「视频编码」选了原始 VVC、播放器却固定为 ExoPlayer 时的说明：ExoPlayer
/// 解不了 VVC，取流改按 H.265。
const exoPlayerVvcHint = 'ExoPlayer 不支持 VVC，选它时按 H.265 播放';

/// 短剧「播放器」（仅 Android；Windows 只有 mpv）。
enum DramaPlayerEngine {
  auto('自动（默认）', 'VVC 用 mpv，其他用 ExoPlayer'),
  exoPlayer('ExoPlayer', '系统硬解，支持悬浮窗；不支持 VVC，按 H.265 播放'),
  mpv('mpv', '全部用 mpv 播放，更耗电；不支持悬浮窗');

  const DramaPlayerEngine(this.label, this.subtitle);

  final String label;
  final String subtitle;
}

class DramaPlayerPreference {
  DramaPlayerPreference._();

  static const _key = 'drama_player_engine';

  static DramaPlayerEngine current = DramaPlayerEngine.auto;

  /// 只有 Android 有两套播放器可选；Windows 固定 mpv。
  static bool get platformSupported =>
      !kIsWeb && defaultTargetPlatform == TargetPlatform.android;

  /// 实际生效的选择：不支持的平台一律按「自动」。
  static DramaPlayerEngine get effective =>
      platformSupported ? current : DramaPlayerEngine.auto;

  static Future<void> load() async {
    try {
      final preferences = await SharedPreferences.getInstance();
      final saved = preferences.getString(_key);
      current = DramaPlayerEngine.values.firstWhere(
        (value) => value.name == saved,
        orElse: () => DramaPlayerEngine.auto,
      );
    } catch (_) {
      current = DramaPlayerEngine.auto;
    }
  }

  static Future<void> set(DramaPlayerEngine value) async {
    final preferences = await SharedPreferences.getInstance();
    if (!await preferences.setString(_key, value.name)) {
      throw StateError('Could not save the drama player engine');
    }
    current = value;
  }

  @visibleForTesting
  static void resetForTest() => current = DramaPlayerEngine.auto;
}

/// 短剧起播路由：全屏播放页与信息流共用，保证两边行为一致。
///
/// - 视频编码 × 播放器 决定取流是否带 `codec=vvc`；
/// - 流本身的编码（Rust 返回的 `codec`）× 播放器 决定用 mpv 还是 ExoPlayer。
///
/// [fallback] 是本会话 mpv 已失败过：之后一律 ExoPlayer + H.265。
abstract final class DramaPlaybackRoute {
  /// 取流是否请求原始 VVC。ExoPlayer 解不了 VVC，固定 ExoPlayer 时改取 H.265。
  static bool requestVvc({required bool fallback}) =>
      !fallback &&
      DramaCodecPreference.vvcActive &&
      DramaPlayerPreference.effective != DramaPlayerEngine.exoPlayer;

  /// 这一路流是否交给 mpv（[NativePlayer.vvc]）。
  static bool useMpv({required bool streamIsVvc, required bool fallback}) {
    if (fallback) return false;
    return switch (DramaPlayerPreference.effective) {
      DramaPlayerEngine.exoPlayer => false,
      DramaPlayerEngine.mpv => true,
      DramaPlayerEngine.auto => streamIsVvc && requestVvc(fallback: false),
    };
  }

  /// mpv 失败退回 ExoPlayer 时的提示：VVC 流退到 H.265，普通流只换播放器。
  static String fallbackToast({required bool streamWasVvc}) =>
      streamWasVvc ? vvcFallbackToast : mpvFallbackToast;
}
