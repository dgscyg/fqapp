import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:fqapp/services/drama_codec_preference.dart';
import 'package:fqapp/services/episode_source_cache.dart';
import 'package:fqapp/services/native_player.dart';
import 'package:fqapp/services/playback_debug.dart';
import 'package:fqapp/widgets/player/playback_debug_overlay.dart';

const _key = '00112233445566778899aabbccddeeff';
const _url =
    'https://v3-cdn.example.invalid/path/secret.mp4?x-signature=abc&expires=1';

EpisodeSource _source({bool vvc = true}) => EpisodeSource(
  _url,
  _key,
  const [
    EpisodeVariant(
      name: '1080P',
      url: _url,
      keyHex: _key,
      height: 1080,
      qualityHeight: 1080,
      isVvc: true,
    ),
  ],
  true,
  null,
  vvc,
);

class _StatsPlayer extends NativePlayer {
  _StatsPlayer(this.stats) : super(vvc: true);

  Map<String, Object?> stats;

  @override
  Future<Map<String, Object?>> debugStats() async => stats;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    SharedPreferences.setMockInitialValues({});
    PlaybackDebugPreference.resetForTest();
    DramaPlayerPreference.resetForTest();
  });

  test('mpv 统计：编码、改写前后文件头、代理吞吐，且不含密钥与地址路径', () {
    final rows = playbackDebugRows(
      PlaybackDebugInfo(
        mpv: true,
        fallback: false,
        requestedVvc: true,
        source: _source(),
        videoId: '7704572447588092953',
        seriesId: '7704572437853113406',
        danmakuCount: 17,
        position: const Duration(seconds: 12),
        duration: const Duration(seconds: 95),
      ),
      {
        'engine': 'mpv',
        'videoFormat': 'vvc',
        'width': '1080',
        'height': '1920',
        'fps': '30.000000',
        'videoBitrate': '2400000',
        'audioCodec': 'aac',
        'audioBitrate': '128000',
        'sampleRate': '44100',
        'channels': '2',
        'container': 'mov,mp4,m4a,3gp,3g2,mj2',
        'decoder': 'no',
        'droppedFrames': '3',
        'renderedFrames': '360',
        'bufferSeconds': '8.5',
        'cacheSpeed': '500000',
        'proxied': true,
        'proxy': {
          'served_bytes': 3145728,
          'encrypted': true,
          'source_fourcc': 'bvc2',
          'output_fourcc': 'vvc1',
        },
      },
      proxyBps: 4000000,
    );
    final text = playbackDebugText(rows);
    expect(text, contains('mpv · 自动：VVC 流用 mpv'));
    expect(text, contains('请求 vvc（codec_type=4）'));
    expect(text, contains('实际 vvc'));
    expect(text, contains('bvc2 → vvc1'));
    expect(text, contains('1080×1920 @ 30 fps · 2.40 Mbps'));
    expect(text, contains('aac / 128 kbps / 44100 Hz / 2 ch'));
    expect(text, contains('libavcodec（软解）'));
    expect(text, contains('dropped 3'));
    expect(text, contains('8.5 s'));
    expect(text, contains('下载 4.00 Mbps'));
    expect(text, contains('代理 4.00 Mbps'));
    expect(text, contains('0:12.0 / 1:35.0'));
    expect(text, contains('CENC 是，密钥已取得'));
    expect(text, contains('清晰度: 1080P'));
    expect(text, contains('video_id: 7704572447588092953'));
    expect(text, contains('series_id: 7704572437853113406'));
    expect(text, contains('视频域名: v3-cdn.example.invalid'));
    expect(text, contains('已加载 17 条'));
    // 不泄露密钥、路径、签名。
    expect(text, isNot(contains(_key)));
    expect(text, isNot(contains('secret.mp4')));
    expect(text, isNot(contains('signature')));
  });

  test('ExoPlayer 统计：MIME 归一为 hevc，硬解，回落原因', () {
    final text = playbackDebugText(
      playbackDebugRows(
        PlaybackDebugInfo(
          mpv: false,
          fallback: true,
          requestedVvc: false,
          source: _source(vvc: false),
        ),
        {
          'engine': 'ExoPlayer',
          'videoMime': 'video/hevc',
          'width': 720,
          'height': 1280,
          'fps': 25.0,
          'videoBitrate': 900000,
          'audioMime': 'audio/mp4a-latm',
          'sampleRate': 44100,
          'decoder': 'c2.qti.hevc.decoder',
          'hardware': true,
          'droppedFrames': 0,
          'renderedFrames': 120,
          'bufferMs': 12000,
          'bandwidthBps': 8000000,
        },
      ),
    );
    expect(text, contains('ExoPlayer · 回落'));
    expect(text, contains('请求 h265（默认）'));
    expect(text, contains('实际 hevc'));
    expect(text, contains('c2.qti.hevc.decoder（硬解）'));
    expect(text, contains('12.0 s'));
    expect(text, contains('带宽估计 8.00 Mbps'));
    expect(text, contains('mp4a-latm'));
  });

  test('原因：设置固定的播放器', () {
    DramaPlayerPreference.current = DramaPlayerEngine.mpv;
    expect(playbackDebugReason(mpv: true, fallback: false), '设置：固定 mpv');
    DramaPlayerPreference.current = DramaPlayerEngine.exoPlayer;
    expect(playbackDebugReason(mpv: false, fallback: false), '设置：固定 ExoPlayer');
  });

  test('streamHost 只留域名', () {
    expect(streamHost(_url), 'v3-cdn.example.invalid');
    expect(streamHost('http://127.0.0.1:5000/media/x'), '127.0.0.1:5000');
    expect(streamHost(null), '—');
  });

  testWidgets('开关打开才显示，复制的文本不含密钥', (tester) async {
    final player = _StatsPlayer({'engine': 'mpv', 'videoFormat': 'hevc'});
    String? copied;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String?;
        }
        return null;
      },
    );
    Widget app() => MaterialApp(
      home: Scaffold(
        body: Stack(
          children: [
            PlaybackDebugOverlay(
              player: player,
              info: () => PlaybackDebugInfo(
                mpv: true,
                fallback: false,
                requestedVvc: false,
                source: _source(vvc: false),
                danmakuCount: 4,
              ),
            ),
          ],
        ),
      ),
    );
    await tester.pumpWidget(app());
    expect(find.byKey(const ValueKey('playback-debug-panel')), findsNothing);

    await PlaybackDebugPreference.set(true);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 10));
    expect(find.byKey(const ValueKey('playback-debug-panel')), findsOneWidget);
    expect(find.textContaining('实际 hevc'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('playback-debug-copy')));
    await tester.pump();
    expect(copied, contains('已加载 4 条'));
    expect(copied, isNot(contains(_key)));
    expect(copied, isNot(contains('x-signature')));

    // 每秒刷新：统计变化后跟着更新。
    player.stats = {'engine': 'mpv', 'videoFormat': 'vvc'};
    await tester.pump(const Duration(seconds: 1));
    await tester.pump();
    expect(find.textContaining('实际 vvc'), findsOneWidget);

    await PlaybackDebugPreference.set(false);
    await tester.pump();
    expect(find.byKey(const ValueKey('playback-debug-panel')), findsNothing);
  });
}
