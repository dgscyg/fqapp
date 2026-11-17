import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:media_kit/media_kit.dart';
import 'package:media_kit_video/media_kit_video.dart';

import 'app_log.dart';
import 'desktop_media.dart';

/// The parts of a player host [MpvPlayers] and the desktop adapter use.
abstract class MpvHost {
  Future<void> open(String url, String key, {int? startMs});
  Future<void> command(String method, Map args);
  Map<String, double> playbackSettings();
  Widget surface({Key? key});
  Future<void> dispose();
}

/// One media_kit (libmpv) player behind the `fqapp/native_player` contract:
/// it emits the same events as the Android ExoPlayer host
/// (created/position/duration/playing/buffering/completed/firstFrame/
/// videoSize/error) and answers play/pause/seek/setRate/setVolume.
///
/// Encrypted media (and, in VVC mode, every remote file) is played through
/// the Rust loopback proxy ([DesktopMedia]): it decrypts CENC and rewrites a
/// bytevc2 wrapper into standard `vvc1`/`vvcC` while streaming.
class MpvPlayerHost implements MpvHost {
  MpvPlayerHost(
    this.id,
    this.emit, {
    this.forceProxy = false,
    VideoControllerConfiguration configuration =
        const VideoControllerConfiguration(),
  }) {
    video = VideoController(player, configuration: configuration);
    // media_kit initializes textures after a frame, including on an idle window.
    if (defaultTargetPlatform == TargetPlatform.linux) {
      WidgetsBinding.instance.scheduleFrame();
    }
    void event(String type, dynamic value) {
      if (!disposed) emit({'playerId': id, 'type': type, 'value': value});
    }

    subscriptions.addAll([
      player.stream.position.listen((v) => event('position', v.inMilliseconds)),
      player.stream.duration.listen((v) => event('duration', v.inMilliseconds)),
      player.stream.playing.listen((v) {
        AppLog.d('desktop-player', 'id=$id playing=$v');
        event('playing', v);
      }),
      player.stream.buffering.listen((v) => event('buffering', v)),
      player.stream.completed.listen((v) => event('completed', v)),
      player.stream.width.listen((_) => _size()),
      player.stream.height.listen((_) => _size()),
      player.stream.error.listen(
        (_) => event('error', {
          'message': '媒体播放失败，请重试或切换清晰度',
          if (codec == 'vvc') 'codec': 'vvc',
        }),
      ),
    ]);
    unawaited(
      video.waitUntilFirstFrameRendered.then((_) => event('firstFrame', true)),
    );
  }

  final int id;
  final void Function(Map<String, dynamic>) emit;

  /// Route clear remote files through the proxy too: a bytevc2 file needs
  /// the wrapper rewrite even when it is not encrypted.
  final bool forceProxy;
  final player = Player();
  late final VideoController video;
  final subscriptions = <StreamSubscription<dynamic>>[];
  final ready = Completer<void>();
  DesktopMedia? media;

  /// `vvc` when the proxy rewrote a bytevc2 track, else `other`/null.
  String? codec;
  bool disposed = false;

  @override
  Future<void> open(String url, String key, {int? startMs}) async {
    ready.future.ignore();
    try {
      var playbackUrl = url;
      final remote = url.startsWith('http://') || url.startsWith('https://');
      if (key.isNotEmpty || (forceProxy && remote)) {
        final session = await DesktopMedia.open(url, key);
        if (disposed) {
          session.close();
          return;
        }
        media = session;
        codec = session.codec;
        playbackUrl = session.url;
      }
      // Plain audio/video follows the original URI player contract. Some
      // public audio origins return 200 only and must not require CENC Range I/O.
      if (disposed) return;
      if (startMs != null && startMs > 0) {
        // The player opens exactly one file, so the start option is never reset.
        final platform = player.platform;
        if (platform is NativePlayer) {
          await platform.setProperty(
            'start',
            (startMs / 1000).toStringAsFixed(3),
          );
        }
      }
      await player.open(Media(playbackUrl), play: false);
      AppLog.d('desktop-player', 'id=$id opened paused codec=$codec');
      if (disposed) return;
      ready.complete();
      emit({'playerId': id, 'type': 'created', 'value': id});
      _size();
    } catch (error) {
      if (!ready.isCompleted) ready.completeError(StateError('无法打开媒体'));
      final value = desktopOpenFailure(id, error);
      if (!disposed) {
        emit({'playerId': id, 'type': 'error', 'value': value});
      }
      media?.close();
    }
  }

  void _size() {
    if (!disposed) {
      emit({
        'playerId': id,
        'type': 'videoSize',
        'width': player.state.width ?? 0,
        'height': player.state.height ?? 0,
        'rotationCorrection': 0,
      });
    }
  }

  @override
  Future<void> command(String method, Map args) async {
    await ready.future;
    if (disposed) return;
    AppLog.d('desktop-player', 'id=$id command=$method');
    switch (method) {
      case 'play':
        await player.play();
        emit({'playerId': id, 'type': 'playWhenReady', 'value': true});
      case 'pause':
        await player.pause();
        emit({'playerId': id, 'type': 'playWhenReady', 'value': false});
      case 'seek':
        await player.seek(
          Duration(milliseconds: (args['positionMs'] as num).toInt()),
        );
      case 'setRate':
        await player.setRate((args['rate'] as num).toDouble());
      case 'setVolume':
        await player.setVolume(
          (args['volume'] as num).toDouble().clamp(0, 1) * 100,
        );
      default:
        throw MissingPluginException(method);
    }
  }

  @override
  Map<String, double> playbackSettings() => {
    'volume': player.state.volume / 100,
    'rate': player.state.rate,
  };

  @override
  Widget surface({Key? key}) => Video(
    key: key,
    controller: video,
    controls: NoVideoControls,
    fit: BoxFit.fill,
    pauseUponEnteringBackgroundMode: false,
    resumeUponEnteringForegroundMode: false,
  );

  @override
  Future<void> dispose() async {
    if (disposed) return;
    disposed = true;
    if (!ready.isCompleted) ready.complete();
    for (final subscription in subscriptions) {
      await subscription.cancel();
    }
    try {
      await player.dispose();
    } finally {
      media?.close();
    }
  }
}

/// Android short-drama players that run on media_kit instead of ExoPlayer
/// (VVC mode). Ids live in their own range so they never collide with the
/// ExoPlayer host's ids; `NativePlayer` routes calls for these ids here.
class MpvPlayers {
  MpvPlayers._();
  static final instance = MpvPlayers._();

  static const _libs = MethodChannel('fqapp/mpv_libs');

  /// Test seam: replaces the platform check and the native library load.
  @visibleForTesting
  static Future<bool> Function()? availabilityOverride;

  /// Test seam: builds the host for an id (a fake in widget tests).
  @visibleForTesting
  static MpvHost Function(int id, void Function(Map<String, dynamic>))?
  hostFactory;

  final _players = <int, MpvHost>{};
  int _nextId = 2000000000;
  Future<bool>? _available;

  /// Event sink, installed by `NativePlayer` (its global dispatcher).
  void Function(Map<String, dynamic>)? sink;

  bool owns(int? id) => id != null && _players.containsKey(id);

  /// Whether the VVC-capable libmpv is packaged and loads. Cached: a failed
  /// load is not retried in this process.
  Future<bool> available() {
    if (availabilityOverride case final override?) return override();
    return _available ??= () async {
      if (kIsWeb || defaultTargetPlatform != TargetPlatform.android) {
        return false;
      }
      try {
        final ok = await _libs.invokeMethod<bool>('prepare') ?? false;
        if (ok) MediaKit.ensureInitialized();
        return ok;
      } catch (error) {
        AppLog.w('mpv-player', 'libmpv unavailable (${error.runtimeType})');
        return false;
      }
    }();
  }

  /// Allocates a player and starts opening [url]; events follow on [sink].
  Future<int> create(String url, String key, {int? startMs}) async {
    if (!await available()) {
      throw PlatformException(code: 'mpv_unavailable', message: '缺少 VVC 播放组件');
    }
    final id = _nextId++;
    void emit(Map<String, dynamic> event) => sink?.call(event);
    final host =
        hostFactory?.call(id, emit) ??
        MpvPlayerHost(id, emit, forceProxy: true);
    _players[id] = host;
    unawaited(host.open(url, key, startMs: startMs));
    return id;
  }

  Future<dynamic> invoke(String method, Map args) async {
    final id = (args['id'] as num?)?.toInt();
    final host = _players[id];
    if (method == 'dispose') {
      _players.remove(id);
      await host?.dispose();
      return null;
    }
    if (host == null) {
      throw PlatformException(code: 'player_closed', message: '播放器已关闭');
    }
    if (method == 'getPlaybackSettings') return host.playbackSettings();
    await host.command(method, args);
    return null;
  }

  Widget surface(int id, {Key? key}) =>
      _players[id]?.surface(key: key) ?? const SizedBox.expand();

  @visibleForTesting
  Future<void> resetForTest() async {
    for (final host in _players.values.toList()) {
      await host.dispose();
    }
    _players.clear();
    _available = null;
    sink = null;
  }
}
