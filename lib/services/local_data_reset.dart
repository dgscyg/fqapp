import 'dart:typed_data';

import 'package:hive_ce/hive.dart';

import 'encrypted_hive.dart';
import 'secure_key_provider.dart';

/// What [LocalDataReset.run] did.
class LocalDataResetResult {
  const LocalDataResetResult({
    required this.resetKeyNames,
    required this.deletedBoxes,
    required this.devicePoolKeyKept,
  });

  /// Secure-storage entries that were deleted and regenerated.
  final List<String> resetKeyNames;

  /// Encrypted boxes (`*_enc`) removed from disk.
  final List<String> deletedBoxes;

  /// True when the device-pool key was still readable and left untouched, so
  /// the encrypted `device_pool.json` (the device identity) stays readable.
  final bool devicePoolKeyKept;
}

/// User-confirmed recovery when the Hive box key cannot be read back from
/// secure storage (Keystore entry undecryptable, read keeps throwing).
///
/// Order matters, so a failure never leaves the user worse off:
/// 1. Probe the device-pool key with a fresh attempt. If it loads, it is
///    kept: the device identity in `device_pool.json` is not touched.
/// 2. Delete and regenerate the broken keys in secure storage: always the
///    Hive box key; the device-pool key only if step 1 failed. If this
///    throws (Keystore still refusing writes), nothing on disk was deleted
///    yet and the original error reaches the caller.
/// 3. Delete every `*_enc` box (history, shelf, caches, download indexes —
///    all unreadable without the old key). Legacy plaintext boxes are kept
///    and migrate into the new encrypted boxes on the next open.
///
/// A reset device-pool key means the old encrypted pool file was already
/// unreadable; the Rust core moves it aside and auto-registers a fresh
/// device, as it does for any empty pool. The new key is handed to a
/// running core via [reprovisionDevicePoolKey].
class LocalDataReset {
  LocalDataReset._();

  static Future<LocalDataResetResult> run({
    HiveInterface? hive,
    SecureKeyProvider? keys,
    Future<void> Function(Uint8List key)? reprovisionDevicePoolKey,
  }) async {
    final box = hive ?? Hive;
    final provider = keys ?? SecureKeyProvider.instance;
    provider.clearFailureCooldown();

    final poolKey = await provider.devicePoolKey();
    final poolKeyKept = poolKey != null;
    final toReset = <String>[
      SecureKeyProvider.hiveBoxKeyName,
      if (!poolKeyKept) SecureKeyProvider.devicePoolKeyName,
    ];
    await provider.resetKeys(toReset);

    final deleted = await EncryptedHive.deleteAllEncryptedBoxes(box);

    if (!poolKeyKept && reprovisionDevicePoolKey != null) {
      final fresh = await provider.devicePoolKey();
      if (fresh != null) await reprovisionDevicePoolKey(fresh);
    }
    return LocalDataResetResult(
      resetKeyNames: toReset,
      deletedBoxes: deleted,
      devicePoolKeyKept: poolKeyKept,
    );
  }
}
