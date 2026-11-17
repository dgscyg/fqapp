import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:fqapp/services/drama_codec_preference.dart';

void main() {
  setUp(() {
    SharedPreferences.setMockInitialValues({});
    DramaCodecPreference.resetForTest();
  });
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    DramaCodecPreference.resetForTest();
  });

  test('默认 H.265，标签与设置项一致', () async {
    await DramaCodecPreference.load();
    expect(DramaCodecPreference.current, DramaVideoCodec.h265);
    expect(DramaCodecPreference.vvcActive, isFalse);
    expect(DramaVideoCodec.h265.label, 'H.265（默认）');
    expect(DramaVideoCodec.vvc.label, '原始 VVC');
  });

  test('选择 VVC 会持久化并在重新读取后保留', () async {
    await DramaCodecPreference.set(DramaVideoCodec.vvc);
    expect(DramaCodecPreference.vvcActive, isTrue);
    final preferences = await SharedPreferences.getInstance();
    expect(preferences.getString('drama_video_codec'), 'vvc');

    DramaCodecPreference.resetForTest();
    await DramaCodecPreference.load();
    expect(DramaCodecPreference.current, DramaVideoCodec.vvc);
  });

  test('未知存储值回落到 H.265', () async {
    SharedPreferences.setMockInitialValues({'drama_video_codec': 'av1'});
    await DramaCodecPreference.load();
    expect(DramaCodecPreference.current, DramaVideoCodec.h265);
  });

  test('只有 Android / Windows 提供 VVC', () async {
    await DramaCodecPreference.set(DramaVideoCodec.vvc);
    for (final platform in TargetPlatform.values) {
      debugDefaultTargetPlatformOverride = platform;
      final supported =
          platform == TargetPlatform.android ||
          platform == TargetPlatform.windows;
      expect(
        DramaCodecPreference.platformSupported,
        supported,
        reason: '$platform',
      );
      expect(DramaCodecPreference.vvcActive, supported, reason: '$platform');
    }
  });
}
