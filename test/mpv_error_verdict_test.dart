import 'package:flutter_test/flutter_test.dart';

import 'package:fqapp/services/mpv_player_host.dart';

/// mpv property reader over a fixed map ('' for anything missing).
Future<String> Function(String) _props(Map<String, String> values) =>
    (name) async => values[name] ?? '';

const _vvcTrack = {
  'idle-active': 'no',
  'track-list/count': '2',
  'track-list/0/type': 'video',
  'track-list/1/type': 'audio',
};

void main() {
  test('libvvdec 优先、其余解码器照旧（尾逗号）', () {
    expect(MpvPlayerHost.preferredDecoders, 'libvvdec,');
  });

  group('mpv error 判定', () {
    test('VVC：原生 vvc 打不开、换 libvvdec 后在解码 → 不算失败（1.0.217 回落的根因）', () async {
      final verdict = await mpvErrorVerdict(
        _props({
          ..._vvcTrack,
          'current-tracks/video/decoder-desc': 'libvvdec H.266 / VVC',
        }),
        firstFrame: false,
      );
      expect(verdict.fatal, isFalse);
      expect(verdict.reason, contains('libvvdec'));
    });

    test('有视频轨但没有任何解码器 → 失败', () async {
      final verdict = await mpvErrorVerdict(
        _props(_vvcTrack),
        firstFrame: false,
      );
      expect(verdict, (fatal: true, reason: 'no video decoder'));
    });

    test('文件没加载起来（idle）→ 失败', () async {
      final verdict = await mpvErrorVerdict(
        _props({'idle-active': 'yes'}),
        firstFrame: false,
      );
      expect(verdict.fatal, isTrue);
    });

    test('网络/文件错误照旧算失败', () async {
      final verdict = await mpvErrorVerdict(
        _props(_vvcTrack),
        firstFrame: true,
        ioError: true,
      );
      expect(verdict, (fatal: true, reason: 'io'));
    });

    test('已出画面后的解码告警不打断播放', () async {
      final verdict = await mpvErrorVerdict(
        _props(_vvcTrack),
        firstFrame: true,
      );
      expect(verdict.fatal, isFalse);
    });

    test('还在打开（没有轨道）→ 先不判失败', () async {
      final verdict = await mpvErrorVerdict(
        _props({'idle-active': 'no'}),
        firstFrame: false,
      );
      expect(verdict.fatal, isFalse);
    });

    test('纯音频：看音频解码器', () async {
      final props = {
        'idle-active': 'no',
        'track-list/count': '2',
        'track-list/0/type': 'video',
        'track-list/0/albumart': 'yes',
        'track-list/1/type': 'audio',
      };
      expect(
        (await mpvErrorVerdict(_props(props), firstFrame: false)).fatal,
        isTrue,
      );
      expect(
        (await mpvErrorVerdict(
          _props({...props, 'current-tracks/audio/decoder-desc': 'aac'}),
          firstFrame: false,
        )).fatal,
        isFalse,
      );
    });
  });

  test('mpv 日志脱敏：地址只留域名，长十六进制打码', () {
    const key = '00112233445566778899aabbccddeeff';
    final text = redactMpvLog(
      'Failed to open https://v3-cdn.example.invalid/a/b.mp4?x-signature=s&k=1 '
      'and http://127.0.0.1:40123/media/abcdef key $key',
    );
    expect(text, contains('https://v3-cdn.example.invalid/…'));
    expect(text, contains('http://127.0.0.1:40123/…'));
    expect(text, isNot(contains('x-signature')));
    expect(text, isNot(contains('b.mp4')));
    expect(text, isNot(contains('/media/abcdef')));
    expect(text, isNot(contains(key)));
    expect(
      redactMpvLog('vd: Could not open codec.'),
      'vd: Could not open codec.',
    );
    expect(redactMpvLog('x' * 500).length, lessThanOrEqualTo(301));
  });
}
