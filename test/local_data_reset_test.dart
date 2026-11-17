import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hive_ce/hive.dart';

import 'package:fqapp/services/encrypted_hive.dart';
import 'package:fqapp/services/local_data_reset.dart';
import 'package:fqapp/services/secure_key_provider.dart';

/// The 1.0.212 field failure: the Hive key entry exists but every read throws
/// (flutter_secure_storage BadPaddingException), so `history_enc` can never
/// be opened again. The bootstrap's 「重置本地加密数据」 runs
/// [LocalDataReset.run]; these tests drive it against a real Hive directory
/// and a fake secure store, never the platform channel.
void main() {
  late Directory dir;
  late _BreakableStorage storage;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('fqapp-local-reset-');
    Hive.init(dir.path);
    storage = _BreakableStorage();
  });

  tearDown(() async {
    await Hive.close();
    await dir.delete(recursive: true);
  });

  SecureKeyProvider newProvider() =>
      SecureKeyProvider(storage: storage, errorRetryDelay: Duration.zero);

  /// A previous session: key available, history + shelf written encrypted,
  /// device-pool key provisioned.
  Future<Uint8List> seedEncryptedData() async {
    final keys = newProvider();
    final history = await EncryptedHive.openBox<dynamic>(
      Hive,
      'history',
      keys: keys,
    );
    await history.put('b1', {'title': '旧历史'});
    final shelf = await EncryptedHive.openBox<dynamic>(
      Hive,
      'shelf',
      keys: keys,
    );
    await shelf.put('s1', {'title': '旧书架'});
    final poolKey = await keys.devicePoolKey();
    await Hive.close();
    return poolKey!;
  }

  test('unreadable key surfaces the real error, reset reopens empty boxes '
      'and keeps the device-pool key', () async {
    final poolKey = await seedEncryptedData();
    final oldHiveKey = storage.values[SecureKeyProvider.hiveBoxKeyName];
    storage.broken.add(SecureKeyProvider.hiveBoxKeyName);

    // Next launch: opening history fails closed with the platform cause.
    final keys = newProvider();
    Object? error;
    try {
      await EncryptedHive.openBox<dynamic>(Hive, 'history', keys: keys);
    } catch (e) {
      error = e;
    }
    expect(error, isA<SecureKeyUnavailableError>());
    final message = (error! as SecureKeyUnavailableError).message;
    expect(message, contains('history_enc 已存在'));
    expect(message, contains('BadPaddingException'));
    expect(message, isNot(contains(oldHiveKey!)), reason: '不能泄露密钥');
    expect(await Hive.boxExists('history_enc'), isTrue, reason: '重置前数据不动');

    var reprovisioned = 0;
    final result = await LocalDataReset.run(
      hive: Hive,
      keys: keys,
      reprovisionDevicePoolKey: (_) async => reprovisioned++,
    );
    expect(result.devicePoolKeyKept, isTrue);
    expect(result.resetKeyNames, [SecureKeyProvider.hiveBoxKeyName]);
    expect(result.deletedBoxes, containsAll(['history_enc', 'shelf_enc']));
    expect(reprovisioned, 0, reason: '设备池密钥没换，不需要重新下发');

    // Device-pool key untouched: the encrypted device_pool.json (device
    // identity) stays readable.
    expect(
      storage.values[SecureKeyProvider.devicePoolKeyName],
      base64Encode(poolKey),
    );
    expect(await keys.devicePoolKey(), poolKey);

    // Startup continues: boxes open encrypted with the new key, empty.
    final newKey = await keys.getOrCreateKey();
    expect(newKey, isNotNull);
    expect(base64Encode(newKey!), isNot(oldHiveKey));
    final history = await EncryptedHive.openBox<dynamic>(
      Hive,
      'history',
      keys: keys,
    );
    expect(history.isEmpty, isTrue);
    await history.put('b2', {'title': '新历史'});
    await Hive.close();

    // And the next launch reads them back with the persisted new key.
    final relaunch = newProvider();
    final reopened = await EncryptedHive.openBox<dynamic>(
      Hive,
      'history',
      keys: relaunch,
    );
    expect((reopened.get('b2') as Map)['title'], '新历史');
  });

  test('when the device-pool key is broken too it is regenerated and handed '
      'to the running core', () async {
    final oldPoolKey = await seedEncryptedData();
    storage.broken
      ..add(SecureKeyProvider.hiveBoxKeyName)
      ..add(SecureKeyProvider.devicePoolKeyName);
    final keys = newProvider();
    expect(await keys.devicePoolKey(), isNull);

    final handed = <Uint8List>[];
    final result = await LocalDataReset.run(
      hive: Hive,
      keys: keys,
      reprovisionDevicePoolKey: (key) async => handed.add(key),
    );
    expect(result.devicePoolKeyKept, isFalse);
    expect(
      result.resetKeyNames,
      containsAll([
        SecureKeyProvider.hiveBoxKeyName,
        SecureKeyProvider.devicePoolKeyName,
      ]),
    );
    expect(handed, hasLength(1));
    expect(handed.single, isNot(oldPoolKey));
    expect(await keys.devicePoolKey(), handed.single);
  });

  test('a failing delete leaves every box on disk', () async {
    await seedEncryptedData();
    storage.broken.add(SecureKeyProvider.hiveBoxKeyName);
    storage.failDeletes = true;
    final keys = newProvider();
    await expectLater(
      LocalDataReset.run(hive: Hive, keys: keys),
      throwsA(isA<PlatformException>()),
    );
    expect(await Hive.boxExists('history_enc'), isTrue);
    expect(await Hive.boxExists('shelf_enc'), isTrue);
  });
}

/// Secure store whose entries can be "broken": reading a broken entry throws
/// the Android BadPaddingException, as flutter_secure_storage 9 does once its
/// Keystore-wrapped AES key no longer matches. Deleting an entry repairs it
/// (a new write uses the plugin's current, working cipher).
class _BreakableStorage implements SecureKeyStorage {
  final Map<String, String> values = {};
  final Set<String> broken = {};
  bool failDeletes = false;

  @override
  Future<String?> read(String key) async {
    if (broken.contains(key)) {
      throw PlatformException(
        code: 'Exception encountered',
        message: 'read',
        details:
            'javax.crypto.BadPaddingException: error:1e000065:Cipher '
            'functions:OPENSSL_internal:BAD_DECRYPT\n'
            '\tat com.it_nomads.fluttersecurestorage.FlutterSecureStorage.read\n',
      );
    }
    return values[key];
  }

  @override
  Future<void> write(String key, String value) async {
    values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    if (failDeletes) {
      throw PlatformException(code: 'Exception encountered', message: 'delete');
    }
    broken.remove(key);
    values.remove(key);
  }
}
