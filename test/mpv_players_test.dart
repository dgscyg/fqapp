import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:fqapp/services/mpv_player_host.dart';
import 'package:fqapp/services/native_player.dart';

/// 不碰 libmpv 的假 host：open 后按 Android ExoPlayer 的事件契约回报。
class _FakeHost implements MpvHost {
  _FakeHost(this.id, this.emit);

  final int id;
  final void Function(Map<String, dynamic>) emit;
  final commands = <String>[];
  String? openedUrl;
  String? openedKey;
  int? openedStartMs;
  bool disposed = false;

  @override
  Future<void> open(String url, String key, {int? startMs}) async {
    openedUrl = url;
    openedKey = key;
    openedStartMs = startMs;
    emit({'playerId': id, 'type': 'created', 'value': id});
  }

  @override
  Future<void> command(String method, Map args) async => commands.add(method);

  @override
  Map<String, double> playbackSettings() => {'rate': 1, 'volume': 1};

  @override
  Future<Map<String, Object?>> debugStats() async => {
    'engine': 'mpv',
    'videoFormat': 'vvc',
  };

  @override
  Widget surface({Key? key}) => SizedBox(key: key);

  @override
  Future<void> dispose() async => disposed = true;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final hosts = <_FakeHost>[];

  setUp(() {
    hosts.clear();
    MpvPlayers.hostFactory = (id, emit) {
      final host = _FakeHost(id, emit);
      hosts.add(host);
      return host;
    };
  });
  tearDown(() async {
    MpvPlayers.availabilityOverride = null;
    MpvPlayers.hostFactory = null;
    await MpvPlayers.instance.resetForTest();
  });

  test('Android 的 VVC 播放器走 media_kit，而不是 ExoPlayer 通道', () async {
    MpvPlayers.availabilityOverride = () async => true;
    final player = NativePlayer(vvc: true);
    expect(player.usesMpv, isTrue);

    await player.create(
      'https://example.invalid/a.mp4',
      '00112233',
      startMs: 4000,
    );
    expect(hosts, hasLength(1));
    final host = hosts.single;
    expect(host.openedUrl, 'https://example.invalid/a.mp4');
    expect(host.openedKey, '00112233');
    expect(host.openedStartMs, 4000);
    expect(MpvPlayers.instance.owns(host.id), isTrue);

    await player.play();
    expect(host.commands, contains('play'));

    await player.dispose();
    expect(host.disposed, isTrue);
    expect(MpvPlayers.instance.owns(host.id), isFalse);
  });

  test('VVC 播放器的进度/播放/缓冲事件驱动弹幕所需的状态', () async {
    // 播放页与 feed 的弹幕调度只读 position / playing / buffering：
    // media_kit 播放器必须和 ExoPlayer 一样把它们送到 NativePlayer。
    MpvPlayers.availabilityOverride = () async => true;
    final player = NativePlayer(vvc: true);
    await player.create('https://example.invalid/a.mp4', '');
    final host = hosts.single;
    final positions = <Duration>[];
    final sub = player.positionStream.listen(positions.add);
    host.emit({'playerId': host.id, 'type': 'playing', 'value': true});
    host.emit({'playerId': host.id, 'type': 'buffering', 'value': false});
    host.emit({'playerId': host.id, 'type': 'position', 'value': 12500});
    await Future<void>.delayed(Duration.zero);
    expect(player.playing, isTrue);
    expect(player.buffering, isFalse);
    expect(player.position, const Duration(milliseconds: 12500));
    expect(positions, [const Duration(milliseconds: 12500)]);
    await sub.cancel();
    await player.dispose();
  });

  test('缺少 VVC 组件时 create 失败，交给页面回落 H.265', () async {
    MpvPlayers.availabilityOverride = () async => false;
    final player = NativePlayer(vvc: true);
    await expectLater(
      player.create('https://example.invalid/a.mp4', ''),
      throwsA(anything),
    );
    expect(hosts, isEmpty);
    await player.dispose();
  });

  test('默认（H.265）播放器不使用 media_kit', () {
    expect(NativePlayer().usesMpv, isFalse);
    expect(NativePlayer(audioOnly: true).usesMpv, isFalse);
  });

  test('未登记的播放器 id 返回已关闭', () async {
    await expectLater(
      MpvPlayers.instance.invoke('play', {'id': 1}),
      throwsA(isA<PlatformException>()),
    );
  });
}
