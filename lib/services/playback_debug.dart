import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'drama_codec_preference.dart';
import 'episode_source_cache.dart';

/// 设置-短剧「播放调试信息」（默认关）：播放时叠一层类似 YouTube
/// 「详细统计信息」的半透明信息框。
class PlaybackDebugPreference {
  PlaybackDebugPreference._();

  static const _key = 'playback_debug_overlay';

  static final ValueNotifier<bool> enabled = ValueNotifier<bool>(false);

  static Future<void> load() async {
    try {
      final preferences = await SharedPreferences.getInstance();
      enabled.value = preferences.getBool(_key) ?? false;
    } catch (_) {
      enabled.value = false;
    }
  }

  static Future<void> set(bool value) async {
    enabled.value = value;
    try {
      final preferences = await SharedPreferences.getInstance();
      await preferences.setBool(_key, value);
    } catch (_) {
      // 持久化失败时本次运行仍按新值显示。
    }
  }

  @visibleForTesting
  static void resetForTest() => enabled.value = false;
}

/// 播放页/信息流在每次刷新时交给调试层的上下文。只放标识与布尔量：
/// 地址只取域名，密钥只记「有没有」，任何时候都不带值。
@immutable
class PlaybackDebugInfo {
  const PlaybackDebugInfo({
    required this.mpv,
    required this.fallback,
    required this.requestedVvc,
    this.source,
    this.videoId = '',
    this.seriesId = '',
    this.danmakuCount = 0,
    this.position = Duration.zero,
    this.duration = Duration.zero,
  });

  /// 当前播放器是否 mpv（media_kit）。
  final bool mpv;

  /// 本会话 mpv 已失败过、退回了 ExoPlayer。
  final bool fallback;

  /// 取流时是否带了 `codec=vvc`。
  final bool requestedVvc;
  final EpisodeSource? source;
  final String videoId;
  final String seriesId;
  final int danmakuCount;
  final Duration position;
  final Duration duration;
}

/// 「为什么用这个播放器」。
String playbackDebugReason({required bool mpv, required bool fallback}) {
  if (fallback) return '回落（mpv 播放失败后改用 ExoPlayer）';
  if (defaultTargetPlatform == TargetPlatform.windows ||
      defaultTargetPlatform == TargetPlatform.linux) {
    return '桌面端只有 mpv';
  }
  return switch (DramaPlayerPreference.effective) {
    DramaPlayerEngine.exoPlayer => '设置：固定 ExoPlayer',
    DramaPlayerEngine.mpv => '设置：固定 mpv',
    DramaPlayerEngine.auto => mpv ? '自动：VVC 流用 mpv' : '自动：非 VVC 流用 ExoPlayer',
  };
}

/// 把 ExoPlayer 的 MIME / mpv 的 video-format 归一成 vvc/hevc/h264/…。
String? normalizeVideoCodec(Object? raw) {
  if (raw is! String || raw.isEmpty) return null;
  final value = raw.toLowerCase();
  if (value.contains('vvc') || value.contains('h266')) return 'vvc';
  if (value.contains('hevc') || value.contains('h265')) return 'hevc';
  if (value.contains('avc') || value.contains('h264')) return 'h264';
  if (value.contains('av01') || value.contains('av1')) return 'av1';
  if (value.contains('vp9')) return 'vp9';
  return value.replaceFirst('video/', '');
}

String _num(Object? raw) => switch (raw) {
  final num value => value.toString(),
  final String value => value,
  _ => '',
};

double? _double(Object? raw) => switch (raw) {
  final num value => value.toDouble(),
  final String value => double.tryParse(value),
  _ => null,
};

String _bitrate(Object? bitsPerSecond) {
  final value = _double(bitsPerSecond);
  if (value == null || value <= 0) return '—';
  if (value >= 1e6) return '${(value / 1e6).toStringAsFixed(2)} Mbps';
  return '${(value / 1e3).toStringAsFixed(0)} kbps';
}

String _clock(Duration value) {
  final seconds = value.inMilliseconds / 1000;
  final m = seconds ~/ 60;
  final s = seconds - m * 60;
  return '$m:${s.toStringAsFixed(1).padLeft(4, '0')}';
}

/// 只留 scheme://host[:port]，去掉路径、查询串与签名。
String streamHost(String? url) {
  if (url == null || url.isEmpty) return '—';
  final uri = Uri.tryParse(url);
  if (uri == null || uri.host.isEmpty) return '—';
  return uri.hasPort ? '${uri.host}:${uri.port}' : uri.host;
}

/// 调试框里的各行（标签, 值）。[stats] 来自 `NativePlayer.debugStats()`，
/// [proxyBps] 是两次刷新间本地代理吞吐（bit/s，mpv 走代理时才有）。
List<(String, String)> playbackDebugRows(
  PlaybackDebugInfo info,
  Map<String, Object?> stats, {
  double? proxyBps,
}) {
  final source = info.source;
  final proxy = stats['proxy'] is Map
      ? Map<String, Object?>.from(stats['proxy'] as Map)
      : const <String, Object?>{};
  final engine = info.mpv ? 'mpv' : 'ExoPlayer';
  final actual =
      normalizeVideoCodec(stats['videoFormat']) ??
      normalizeVideoCodec(stats['videoMime']) ??
      normalizeVideoCodec(stats['videoCodecs']);
  final streamCodec = source == null
      ? '—'
      : source.isVvc
      ? 'vvc（bytevc2）'
      : '非 VVC';
  final fourccFrom = proxy['source_fourcc'];
  final fourccTo = proxy['output_fourcc'];
  final fourcc = fourccFrom is String && fourccTo is String
      ? '$fourccFrom → $fourccTo（本地代理改写）'
      : info.mpv && stats['proxied'] == true
      ? '未改写'
      : '—';

  final width = _num(stats['width']);
  final height = _num(stats['height']);
  final fps = _double(stats['fps']) ?? _double(stats['displayFps']);

  final decoder = stats['decoder'];
  final String decoderText;
  if (info.mpv) {
    final hw = decoder is String && decoder.isNotEmpty && decoder != 'no';
    final name = stats['decoderName'];
    final software = name is String && name.isNotEmpty ? name : 'libavcodec';
    decoderText = hw ? '$decoder（硬解）' : '$software（软解）';
  } else if (decoder is String && decoder.isNotEmpty) {
    final hw = stats['hardware'];
    decoderText = '$decoder（${hw == false ? '软解' : '硬解'}）';
  } else {
    decoderText = '—';
  }

  final audioCodec =
      stats['audioCodec'] ?? stats['audioMime'] ?? stats['audioCodecs'];
  final sampleRate = _num(stats['sampleRate']);
  final channels = _num(stats['channels']);
  final audio = [
    if (audioCodec is String && audioCodec.isNotEmpty)
      audioCodec.replaceFirst('audio/', ''),
    if (_double(stats['audioBitrate']) != null) _bitrate(stats['audioBitrate']),
    if (sampleRate.isNotEmpty) '$sampleRate Hz',
    if (channels.isNotEmpty) '$channels ch',
  ].join(' / ');

  final dropped = _num(stats['droppedFrames']);
  final decoderDropped = _num(stats['decoderDroppedFrames']);
  final rendered = _num(stats['renderedFrames']);
  final frames = [
    'dropped ${dropped.isEmpty ? '—' : dropped}'
        '${decoderDropped.isEmpty ? '' : '+$decoderDropped（解码）'}',
    'rendered ${rendered.isEmpty ? '—' : rendered}',
  ].join(' / ');

  final bufferSeconds =
      _double(stats['bufferSeconds']) ??
      (_double(stats['bufferMs']) != null
          ? _double(stats['bufferMs'])! / 1000
          : null);

  final speeds = <String>[
    if (_double(stats['bandwidthBps']) != null)
      '带宽估计 ${_bitrate(stats['bandwidthBps'])}',
    if (_double(stats['cacheSpeed']) != null)
      '下载 ${_bitrate(_double(stats['cacheSpeed'])! * 8)}',
    if (proxyBps != null) '代理 ${_bitrate(proxyBps)}',
  ];
  final served = _double(proxy['served_bytes']);
  if (served != null) {
    speeds.add('已送出 ${(served / (1024 * 1024)).toStringAsFixed(1)} MiB');
  }

  final quality = source == null
      ? '—'
      : source.variants
                .where((variant) => variant.url == source.url)
                .map((variant) => variant.name)
                .firstOrNull ??
            '默认';

  final encrypted = source == null
      ? '—'
      : source.keyHex.isNotEmpty
      ? 'CENC 是，密钥已取得'
      : (proxy['encrypted'] == true ? 'CENC 是，密钥缺失' : '否（明文）');

  return [
    (
      '播放器',
      '$engine · ${playbackDebugReason(mpv: info.mpv, fallback: info.fallback)}',
    ),
    (
      '编码',
      '请求 ${info.requestedVvc ? 'vvc（codec_type=4）' : 'h265（默认）'}'
          ' · 流 $streamCodec · 实际 ${actual ?? '—'}',
    ),
    ('文件头', fourcc),
    (
      '画面',
      '${width.isEmpty ? '—' : width}×${height.isEmpty ? '—' : height}'
          '${fps == null ? '' : ' @ ${fps.toStringAsFixed(fps % 1 == 0 ? 0 : 2)} fps'}'
          ' · ${_bitrate(stats['videoBitrate'])}',
    ),
    ('音频', audio.isEmpty ? '—' : audio),
    ('封装', _num(stats['container']).isEmpty ? '—' : _num(stats['container'])),
    ('清晰度', quality),
    ('解码器', decoderText),
    ('帧', frames),
    (
      '缓冲',
      bufferSeconds == null ? '—' : '${bufferSeconds.toStringAsFixed(1)} s',
    ),
    ('网络', speeds.isEmpty ? '—' : speeds.join(' · ')),
    ('进度', '${_clock(info.position)} / ${_clock(info.duration)}'),
    ('加密', encrypted),
    ('video_id', info.videoId.isEmpty ? '—' : info.videoId),
    ('series_id', info.seriesId.isEmpty ? '—' : info.seriesId),
    ('视频域名', streamHost(source?.url)),
    ('弹幕', '已加载 ${info.danmakuCount} 条'),
  ];
}

/// 复制用的纯文本。只由 [playbackDebugRows] 的结果拼出，天然不含地址
/// 路径、签名与密钥。
String playbackDebugText(List<(String, String)> rows) =>
    rows.map((row) => '${row.$1}: ${row.$2}').join('\n');
