import 'dart:async';
import 'dart:typed_data';

import 'app_log.dart';

/// Retries the device-pool key after the platform key store failed at
/// startup, and hands it to the running core once it loads.
///
/// Without the key the Rust core cannot read the encrypted device pool. It
/// stays *locked* - no automatic registration, no save over the file - and
/// reloads the existing identity on the first request after the key arrives.
/// This class is what eventually delivers that key: each attempt runs after
/// the provider's 30 s failure cooldown, so it really reads the key store
/// again instead of replaying the cached failure.
class DevicePoolKeyRetry {
  DevicePoolKeyRetry({
    required this.loadKey,
    required this.provision,
    this.delays = defaultDelays,
  });

  /// After the 30 s failure cooldown, then backing off. A key store that is
  /// still failing after the last attempt needs the user (重试 / 重置本地加密数据).
  static const defaultDelays = <Duration>[
    Duration(seconds: 31),
    Duration(minutes: 2),
    Duration(minutes: 5),
    Duration(minutes: 15),
  ];

  final Future<Uint8List?> Function() loadKey;
  final Future<void> Function(Uint8List key) provision;
  final List<Duration> delays;

  Future<bool>? _running;

  /// Starts the retry loop once; later calls join the running loop.
  /// Completes with whether a key was delivered.
  Future<bool> start() => _running ??= _run();

  Future<bool> _run() async {
    for (var i = 0; i < delays.length; i++) {
      await Future<void>.delayed(delays[i]);
      Uint8List? key;
      try {
        key = await loadKey();
      } catch (error) {
        AppLog.w('backend', '设备池密钥重试 ${i + 1}/${delays.length} 失败：$error');
        continue;
      }
      if (key == null) {
        AppLog.w('backend', '设备池密钥重试 ${i + 1}/${delays.length}：密钥仍不可用');
        continue;
      }
      try {
        await provision(key);
      } catch (error) {
        AppLog.w('backend', '设备池密钥交付失败：$error');
        continue;
      }
      AppLog.i('backend', '设备池密钥已就绪，核心将重新载入原有设备');
      return true;
    }
    AppLog.w('backend', '设备池密钥多次重试仍不可用，设备池保持锁定（不会注册新设备）');
    return false;
  }
}
