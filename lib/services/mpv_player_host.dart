import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:media_kit/media_kit.dart';
import 'package:media_kit_video/media_kit_video.dart';
// The Android controller is not exported, but its --wid listener has to be
// detached before the player is disposed (see [MpvPlayerHost.dispose]).
// ignore: implementation_imports
import 'package:media_kit_video/src/video_controller/android_video_controller/real.dart'
    show AndroidVideoController;

import 'app_log.dart';
import 'desktop_media.dart';

/// The parts of a player host [MpvPlayers] and the desktop adapter use.
abstract class MpvHost {
  Future<void> open(String url, String key, {int? startMs});
  Future<void> command(String method, Map args);
  Map<String, double> playbackSettings();

  /// Snapshot for the playback debug overlay (no URL, no key).
  Future<Map<String, Object?>> debugStats();
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
        if (!v) event('buffering', false);
      }),
      // ExoPlayer 契约：buffering 只表示「想播但卡在缓冲」。media_kit 把
      // 暂停时的 core-idle 也报成 buffering=true，照搬会让播放页在暂停时
      // 挂加载圈、弹幕层与弹幕调度把这段当成缓冲。暂停时一律报 false。
      player.stream.buffering.listen(
        (v) => event('buffering', v && player.state.playing),
      ),
      player.stream.completed.listen((v) => event('completed', v)),
      player.stream.width.listen((_) => _size()),
      player.stream.height.listen((_) => _size()),
      // media_kit 把 vd/ad/stream/cplayer/file 前缀的**每一条** error 级
      // 日志都当成 error 发出来，其中不少并不致命：VVCEasy 的 FFmpeg 里
      // 原生 vvc 解码器是 experimental，mpv 先试它，报
      // 「vd: Could not open codec.」后自己换 libvvdec 照常播放。1.0.217
      // 把这条当成失败，于是 VVC 每次都回落。现在先记下原文，稍后看 mpv
      // 是否真的放不了（空闲、或没有可用解码器）再报失败。
      player.stream.error.listen(_onMpvError),
      player.stream.log.listen(_onMpvLog),
    ]);
    unawaited(
      video.waitUntilFirstFrameRendered.then((_) {
        firstFrame = true;
        event('firstFrame', true);
        unawaited(_logPlaybackPath());
      }),
    );
  }

  /// How long an mpv error may settle before it is judged: mpv tries the
  /// next decoder right away, so a recoverable error is resolved by then.
  @visibleForTesting
  static Duration errorSettle = const Duration(milliseconds: 800);

  /// mpv log lines (warn and above) copied to the app log, per player.
  static const _maxLogLines = 80;
  static bool _versionsLogged = false;

  bool firstFrame = false;
  int _logLines = 0;
  String? _pendingError;
  Timer? _errorCheck;
  bool _failed = false;
  String? _lastErrorPrefix;

  void _onMpvLog(PlayerLog log) {
    if (log.level == 'error') _lastErrorPrefix = log.prefix;
    if (disposed || _logLines >= _maxLogLines) return;
    if (log.level != 'warn' && log.level != 'error' && log.level != 'fatal') {
      return;
    }
    _logLines++;
    AppLog.w(
      'mpv',
      'id=$id ${log.level} [${log.prefix}] ${redactMpvLog(log.text)}',
    );
  }

  void _onMpvError(String text) {
    if (disposed || _failed) return;
    _pendingError = redactMpvLog(text);
    _errorCheck ??= Timer(errorSettle, () => unawaited(_judgeError()));
  }

  Future<void> _judgeError() async {
    _errorCheck = null;
    final text = _pendingError ?? '';
    if (disposed || _failed) return;
    final verdict = await mpvErrorVerdict(
      _property,
      firstFrame: firstFrame,
      ioError:
          const {'stream', 'file'}.contains(_lastErrorPrefix) ||
          text.startsWith('tcp:'),
    );
    if (disposed || _failed) return;
    AppLog.w(
      'mpv',
      'id=$id error "$text" -> ${verdict.fatal ? 'fatal' : 'recovered'} '
          '(${verdict.reason})',
    );
    if (!verdict.fatal) return;
    _failed = true;
    emit({
      'playerId': id,
      'type': 'error',
      'value': {
        'message': '媒体播放失败，请重试或切换清晰度',
        'mpvError': text,
        if (codec == 'vvc') 'codec': 'vvc',
      },
    });
  }

  /// Reads one mpv property as text ('' when unavailable).
  Future<String> _property(String name) async {
    final platform = player.platform;
    if (disposed || platform is! NativePlayer) return '';
    try {
      return await platform.getProperty(name, waitForInitialization: false);
    } catch (_) {
      return '';
    }
  }

  Future<void> _logVersions() async {
    if (_versionsLogged) return;
    _versionsLogged = true;
    final mpv = await _property('mpv-version');
    final ffmpeg = await _property('ffmpeg-version');
    AppLog.i('mpv', 'libmpv $mpv, FFmpeg $ffmpeg, vd=$preferredDecoders');
  }

  Future<void> _logPlaybackPath() async {
    final decoder = await _property('current-tracks/video/decoder-desc');
    final format = await _property('video-format');
    final hwdec = await _property('hwdec-current');
    final vo = await _property('current-vo');
    AppLog.i(
      'mpv',
      'id=$id first frame codec=$codec format=$format decoder="$decoder" '
          'hwdec=$hwdec vo=$vo',
    );
  }

  final int id;
  final void Function(Map<String, dynamic>) emit;

  /// Route clear remote files through the proxy too: a bytevc2 file needs
  /// the wrapper rewrite even when it is not encrypted.
  final bool forceProxy;
  final player = Player(
    configuration: const PlayerConfiguration(logLevel: MPVLogLevel.warn),
  );

  /// mpv `--vd`: libvvdec before the experimental native vvc decoder.
  static const preferredDecoders = 'libvvdec,';
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
      final platform = player.platform;
      if (platform is NativePlayer) {
        // libvvdec first for VVC (the native FFmpeg vvc decoder is
        // experimental and refuses to open); the trailing comma keeps every
        // other decoder (H.264/H.265, hwdec) available as before.
        await platform.setProperty('vd', preferredDecoders);
        unawaited(_logVersions());
      }
      if (startMs != null && startMs > 0 && platform is NativePlayer) {
        // The player opens exactly one file, so the start option is never reset.
        await platform.setProperty(
          'start',
          (startMs / 1000).toStringAsFixed(3),
        );
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

  /// mpv property names read for the playback debug overlay.
  static const debugProperties = {
    'videoFormat': 'video-format',
    'videoCodec': 'video-codec',
    'width': 'video-params/w',
    'height': 'video-params/h',
    'fps': 'container-fps',
    'displayFps': 'estimated-vf-fps',
    'videoBitrate': 'video-bitrate',
    'audioCodec': 'audio-codec-name',
    'audioBitrate': 'audio-bitrate',
    'sampleRate': 'audio-params/samplerate',
    'channels': 'audio-params/channel-count',
    'container': 'file-format',
    'decoder': 'hwdec-current',
    'decoderName': 'current-tracks/video/decoder-desc',
    'droppedFrames': 'frame-drop-count',
    'decoderDroppedFrames': 'decoder-frame-drop-count',
    'renderedFrames': 'estimated-frame-number',
    'bufferSeconds': 'demuxer-cache-duration',
    'cacheSpeed': 'cache-speed',
  };

  @override
  Future<Map<String, Object?>> debugStats() async {
    final values = <String, Object?>{'engine': 'mpv'};
    final platform = player.platform;
    if (disposed || platform is! NativePlayer) return values;
    for (final MapEntry(:key, :value) in debugProperties.entries) {
      try {
        final text = await platform.getProperty(
          value,
          waitForInitialization: false,
        );
        if (text.isNotEmpty) values[key] = text;
      } catch (_) {
        // Property unavailable for this file (e.g. no audio track).
      }
    }
    final proxy = media?.stats() ?? const {};
    if (proxy.isNotEmpty) values['proxy'] = proxy;
    values['proxied'] = media != null;
    values['rewrittenCodec'] = codec;
    return values;
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
    _errorCheck?.cancel();
    if (!ready.isCompleted) ready.complete();
    for (final subscription in subscriptions) {
      await subscription.cancel();
    }
    try {
      await _detachVideoOutput();
      await player.dispose();
    } catch (error) {
      AppLog.w('mpv', 'id=$id dispose failed (${error.runtimeType})');
    } finally {
      media?.close();
    }
  }

  /// media_kit's Android controller re-applies `--wid` and then seeks the
  /// player whenever the surface changes. `Player.dispose` marks the player
  /// disposed *before* it removes that listener, so a surface change in
  /// between (the Video widget leaving the tree) seeked a disposed player:
  /// 「[Player] has been disposed」 as an uncaught async error. Detach the
  /// listener and let an in-flight update finish first.
  Future<void> _detachVideoOutput() async {
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.android) return;
    final controller = video.notifier.value;
    if (controller is! AndroidVideoController) return;
    controller.wid.removeListener(controller.widListener);
    try {
      await controller.lock
          .synchronized(() {})
          .timeout(const Duration(seconds: 2));
    } catch (_) {
      // A stuck surface update must not block disposal.
    }
  }
}

/// Whether an mpv error left the player unable to play.
typedef MpvErrorVerdict = ({bool fatal, String reason});

/// Judges an mpv error from the player state a moment later. Fatal for
/// network/file errors, when mpv went idle (the file failed to load), or
/// when it has a video/audio track but no decoder for it and no frame was
/// ever shown. A decoder that failed to open but was replaced by the next
/// one in the list (VVC: vvc → libvvdec) is not an error.
@visibleForTesting
Future<MpvErrorVerdict> mpvErrorVerdict(
  Future<String> Function(String name) property, {
  required bool firstFrame,
  bool ioError = false,
}) async {
  // Network/file errors keep their old meaning: the stream is gone.
  if (ioError) return (fatal: true, reason: 'io');
  if (await property('idle-active') == 'yes') {
    return (fatal: true, reason: 'idle');
  }
  if (firstFrame) return (fatal: false, reason: 'playing');
  final count = int.tryParse(await property('track-list/count')) ?? 0;
  var video = false;
  for (var i = 0; i < count && i < 16; i++) {
    if (await property('track-list/$i/type') == 'video' &&
        await property('track-list/$i/albumart') != 'yes') {
      video = true;
      break;
    }
  }
  if (count == 0) return (fatal: false, reason: 'loading');
  final kind = video ? 'video' : 'audio';
  final decoder = await property('current-tracks/$kind/decoder-desc');
  if (decoder.isEmpty) return (fatal: true, reason: 'no $kind decoder');
  return (fatal: false, reason: '$kind decoder "$decoder"');
}

final _urlPattern = RegExp(r"[a-zA-Z][a-zA-Z0-9+.-]*://[^\s'\x22<>]+");
final _hexRun = RegExp(r'\b[0-9a-fA-F]{32,}\b');

/// mpv log text with every URL cut to scheme://host[:port] (no path, query
/// or signature) and long hex runs (key material) masked.
@visibleForTesting
String redactMpvLog(String text) {
  final cut = text.replaceAllMapped(_urlPattern, (m) {
    final uri = Uri.tryParse(m[0]!);
    if (uri == null || uri.host.isEmpty) return '<url>';
    return '${uri.scheme}://${uri.host}${uri.hasPort ? ':${uri.port}' : ''}/…';
  });
  final masked = cut.replaceAll(_hexRun, '<hex>');
  return masked.length > 300 ? '${masked.substring(0, 300)}…' : masked;
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
    if (method == 'getDebugStats') return host.debugStats();
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
