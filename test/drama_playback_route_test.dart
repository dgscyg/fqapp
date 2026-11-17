import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:fqapp/services/drama_codec_preference.dart';

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
    DramaCodecPreference.resetForTest();
    DramaPlayerPreference.resetForTest();
  });
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    DramaCodecPreference.resetForTest();
    DramaPlayerPreference.resetForTest();
  });

  // 视频编码 × 播放器 × 流编码 → (取流带 codec=vvc, 用 mpv)。
  const matrix = <(DramaVideoCodec, DramaPlayerEngine, bool, bool, bool)>[
    // codec, engine, streamIsVvc, requestVvc, useMpv
    (DramaVideoCodec.h265, DramaPlayerEngine.auto, false, false, false),
    (DramaVideoCodec.h265, DramaPlayerEngine.auto, true, false, false),
    (DramaVideoCodec.h265, DramaPlayerEngine.exoPlayer, false, false, false),
    (DramaVideoCodec.h265, DramaPlayerEngine.exoPlayer, true, false, false),
    (DramaVideoCodec.h265, DramaPlayerEngine.mpv, false, false, true),
    (DramaVideoCodec.h265, DramaPlayerEngine.mpv, true, false, true),
    (DramaVideoCodec.vvc, DramaPlayerEngine.auto, false, true, false),
    (DramaVideoCodec.vvc, DramaPlayerEngine.auto, true, true, true),
    // ExoPlayer 解不了 VVC：改取 H.265，始终 ExoPlayer。
    (DramaVideoCodec.vvc, DramaPlayerEngine.exoPlayer, false, false, false),
    (DramaVideoCodec.vvc, DramaPlayerEngine.exoPlayer, true, false, false),
    (DramaVideoCodec.vvc, DramaPlayerEngine.mpv, false, true, true),
    (DramaVideoCodec.vvc, DramaPlayerEngine.mpv, true, true, true),
  ];

  for (final (codec, engine, streamVvc, requestVvc, useMpv) in matrix) {
    test('Android ${codec.name} × ${engine.name} × '
        '${streamVvc ? 'vvc' : 'other'} 流', () {
      DramaCodecPreference.current = codec;
      DramaPlayerPreference.current = engine;
      expect(DramaPlaybackRoute.requestVvc(fallback: false), requestVvc);
      expect(
        DramaPlaybackRoute.useMpv(streamIsVvc: streamVvc, fallback: false),
        useMpv,
      );
      // mpv 失败过：一律 ExoPlayer + H.265。
      expect(DramaPlaybackRoute.requestVvc(fallback: true), isFalse);
      expect(
        DramaPlaybackRoute.useMpv(streamIsVvc: streamVvc, fallback: true),
        isFalse,
      );
    });
  }

  test('Windows 没有「播放器」选项：存着的 ExoPlayer 不影响 VVC 取流', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.windows;
    DramaCodecPreference.current = DramaVideoCodec.vvc;
    DramaPlayerPreference.current = DramaPlayerEngine.exoPlayer;
    expect(DramaPlayerPreference.platformSupported, isFalse);
    expect(DramaPlayerPreference.effective, DramaPlayerEngine.auto);
    expect(DramaPlaybackRoute.requestVvc(fallback: false), isTrue);
    expect(
      DramaPlaybackRoute.useMpv(streamIsVvc: true, fallback: false),
      isTrue,
    );
  });

  test('回落提示：VVC 流退 H.265，普通流只换播放器', () {
    expect(
      DramaPlaybackRoute.fallbackToast(streamWasVvc: true),
      vvcFallbackToast,
    );
    expect(
      DramaPlaybackRoute.fallbackToast(streamWasVvc: false),
      mpvFallbackToast,
    );
  });

  test('播放器设置持久化，默认自动；mpv 选项注明没有悬浮窗', () async {
    await DramaPlayerPreference.load();
    expect(DramaPlayerPreference.current, DramaPlayerEngine.auto);
    await DramaPlayerPreference.set(DramaPlayerEngine.mpv);
    final preferences = await SharedPreferences.getInstance();
    expect(preferences.getString('drama_player_engine'), 'mpv');
    DramaPlayerPreference.resetForTest();
    await DramaPlayerPreference.load();
    expect(DramaPlayerPreference.current, DramaPlayerEngine.mpv);
    expect(DramaPlayerEngine.mpv.subtitle, contains('不支持悬浮窗'));
    expect(DramaPlayerEngine.exoPlayer.subtitle, contains('H.265'));
  });
}
