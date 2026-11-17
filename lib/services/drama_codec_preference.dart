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
