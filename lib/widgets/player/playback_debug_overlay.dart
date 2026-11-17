import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';

import '../../services/native_player.dart';
import '../../services/playback_debug.dart';

/// 「播放调试信息」叠层：开关打开时在画面左上角每秒刷新一次。
///
/// 统计来自 [NativePlayer.debugStats]（ExoPlayer 的格式/解码器/缓冲，或
/// mpv 属性 + 本地代理计数），上下文由宿主页通过 [info] 现取。复制的
/// 文本与屏幕上的一致，不含地址路径、签名或密钥。
class PlaybackDebugOverlay extends StatelessWidget {
  const PlaybackDebugOverlay({
    super.key,
    required this.player,
    required this.info,
    this.top = 8,
  });

  final NativePlayer? player;
  final PlaybackDebugInfo Function() info;
  final double top;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<bool>(
    valueListenable: PlaybackDebugPreference.enabled,
    builder: (context, enabled, _) => enabled
        ? Positioned(
            left: 8,
            top: top,
            child: PlaybackDebugPanel(player: player, info: info),
          )
        : const SizedBox.shrink(),
  );
}

class PlaybackDebugPanel extends StatefulWidget {
  const PlaybackDebugPanel({
    super.key,
    required this.player,
    required this.info,
    this.interval = const Duration(seconds: 1),
  });

  final NativePlayer? player;
  final PlaybackDebugInfo Function() info;
  final Duration interval;

  @override
  State<PlaybackDebugPanel> createState() => _PlaybackDebugPanelState();
}

class _PlaybackDebugPanelState extends State<PlaybackDebugPanel> {
  Timer? _timer;
  List<(String, String)> _rows = const [];
  Map<String, Object?> _stats = const {};
  double? _lastServed;
  DateTime? _lastServedAt;
  double? _proxyBps;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _rebuildRows();
    unawaited(_refresh());
    _timer = Timer.periodic(widget.interval, (_) => unawaited(_refresh()));
  }

  @override
  void didUpdateWidget(PlaybackDebugPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(oldWidget.player, widget.player)) {
      _stats = const {};
      _lastServed = null;
      _lastServedAt = null;
      _proxyBps = null;
      _rebuildRows();
      unawaited(_refresh());
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  Future<void> _refresh() async {
    if (_busy) return;
    _busy = true;
    try {
      final player = widget.player;
      final stats = player == null
          ? const <String, Object?>{}
          : await player.debugStats();
      if (!mounted || !identical(player, widget.player)) return;
      _stats = stats;
      _updateThroughput(stats);
      setState(_rebuildRows);
    } finally {
      _busy = false;
    }
  }

  void _updateThroughput(Map<String, Object?> stats) {
    final proxy = stats['proxy'];
    final served = proxy is Map ? proxy['served_bytes'] : null;
    if (served is! num) {
      _proxyBps = null;
      return;
    }
    final now = DateTime.now();
    final last = _lastServed;
    final lastAt = _lastServedAt;
    if (last != null && lastAt != null) {
      final seconds = now.difference(lastAt).inMilliseconds / 1000;
      if (seconds > 0) _proxyBps = (served - last) * 8 / seconds;
    }
    _lastServed = served.toDouble();
    _lastServedAt = now;
  }

  void _rebuildRows() {
    _rows = playbackDebugRows(widget.info(), _stats, proxyBps: _proxyBps);
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: playbackDebugText(_rows)));
    if (!mounted) return;
    ScaffoldMessenger.maybeOf(
      context,
    )?.showSnackBar(const SnackBar(content: Text('已复制播放调试信息')));
  }

  @override
  Widget build(BuildContext context) {
    const style = TextStyle(
      color: Colors.white,
      fontSize: 10.5,
      height: 1.35,
      fontFamily: 'monospace',
      fontFeatures: [FontFeature.tabularFigures()],
    );
    return ConstrainedBox(
      key: const ValueKey('playback-debug-panel'),
      constraints: const BoxConstraints(maxWidth: 340),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: Colors.black.withValues(alpha: 0.62),
          borderRadius: BorderRadius.circular(6),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(8, 4, 2, 6),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Text(
                    '播放调试信息',
                    style: TextStyle(
                      color: Colors.white,
                      fontSize: 11,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(width: 12),
                  IconButton(
                    key: const ValueKey('playback-debug-copy'),
                    tooltip: '复制',
                    visualDensity: VisualDensity.compact,
                    iconSize: 16,
                    color: Colors.white,
                    onPressed: _copy,
                    icon: const Icon(Icons.copy_rounded),
                  ),
                ],
              ),
              IgnorePointer(
                child: Text(
                  playbackDebugText(_rows),
                  key: const ValueKey('playback-debug-text'),
                  style: style,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
