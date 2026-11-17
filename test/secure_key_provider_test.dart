import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:fqapp/services/secure_key_provider.dart';

/// The bootstrap awaits this provider during startup, so a secure-storage
/// call that never resolves must degrade (null) instead of hanging startup
/// forever — a wedged Keystore, or an unmocked platform channel in widget
/// tests, both present as a Future that never completes.
void main() {
  test('Linux never replaces a malformed persisted key', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final storage = InMemorySecureKeyStorage();
    await storage.write(SecureKeyProvider.hiveBoxKeyName, 'malformed');
    expect(await SecureKeyProvider(storage: storage).getOrCreateKey(), isNull);
    expect(await storage.read(SecureKeyProvider.hiveBoxKeyName), 'malformed');
  });

  test('a never-resolving read degrades to null after the timeout', () async {
    final provider = SecureKeyProvider(
      storage: _HangingStorage(),
      storageTimeout: const Duration(milliseconds: 100),
    );
    final stopwatch = Stopwatch()..start();
    expect(await provider.getOrCreateKey(), isNull);
    expect(stopwatch.elapsedMilliseconds, lessThan(5000));
  });

  test(
    'a failed load is cached: the next lookup returns null at once',
    () async {
      final provider = SecureKeyProvider(
        storage: _HangingStorage(),
        storageTimeout: const Duration(milliseconds: 100),
      );
      expect(await provider.getOrCreateKey(), isNull);
      final stopwatch = Stopwatch()..start();
      expect(await provider.getOrCreateKey(), isNull);
      // Would take another full timeout if the failure were not cached.
      expect(stopwatch.elapsedMilliseconds, lessThan(50));
    },
  );

  test('a throwing store degrades to null', () async {
    final provider = SecureKeyProvider(
      storage: _ThrowingStorage(),
      storageTimeout: const Duration(milliseconds: 100),
    );
    expect(await provider.getOrCreateKey(), isNull);
  });

  test('a working store generates once and reuses the persisted key', () async {
    final storage = _MapStorage();
    final first = SecureKeyProvider(
      storage: storage,
      storageTimeout: const Duration(milliseconds: 100),
    );
    final key = await first.getOrCreateKey();
    expect(key, isNotNull);
    expect(key!.length, 32);
    expect(await first.getOrCreateKey(), same(key));

    // A fresh provider reads the persisted key instead of generating anew.
    final second = SecureKeyProvider(
      storage: storage,
      storageTimeout: const Duration(milliseconds: 100),
    );
    expect(await second.getOrCreateKey(), key);
  });

  test('a read that times out once is retried before giving up', () async {
    final storage = _FlakyStorage(hangFirstReads: 1)
      ..values[SecureKeyProvider.hiveBoxKeyName] =
          'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=';
    final provider = SecureKeyProvider(
      storage: storage,
      storageTimeout: const Duration(milliseconds: 100),
    );
    final key = await provider.getOrCreateKey();
    expect(key, isNotNull);
    expect(key!.first, 0);
    expect(key.last, 31);
    expect(storage.reads, 2);
    // The existing key is reused, never replaced by a fresh one.
    expect(storage.writes, 0);
  });

  test(
    'two timed-out reads still degrade to null (no endless retry)',
    () async {
      final storage = _FlakyStorage(hangFirstReads: 2);
      final provider = SecureKeyProvider(
        storage: storage,
        storageTimeout: const Duration(milliseconds: 100),
      );
      expect(await provider.getOrCreateKey(), isNull);
      expect(storage.reads, 2);
      expect(storage.writes, 0);
    },
  );

  test('clearFailureCooldown lets a manual retry ask storage again', () async {
    final storage = _FlakyStorage(hangFirstReads: 2);
    final provider = SecureKeyProvider(
      storage: storage,
      storageTimeout: const Duration(milliseconds: 100),
    );
    expect(await provider.getOrCreateKey(), isNull);
    // Within the cooldown a lookup does not touch storage at all.
    expect(await provider.getOrCreateKey(), isNull);
    expect(storage.reads, 2);
    provider.clearFailureCooldown();
    final key = await provider.getOrCreateKey();
    expect(key, isNotNull);
    expect(storage.reads, 3);
  });

  test('concurrent first-launch requests get one key and one write', () async {
    final storage = _SlowStorage();
    final provider = SecureKeyProvider(
      storage: storage,
      errorRetryDelay: Duration.zero,
    );
    final keys = await Future.wait([
      for (var i = 0; i < 5; i++) provider.getOrCreateKey(),
    ]);
    expect(keys.first, isNotNull);
    for (final key in keys) {
      expect(key, keys.first);
    }
    expect(storage.writes, 1);
    expect(storage.reads, 1);
  });

  test('two providers over the same storage share one load', () async {
    // The 1.0.212 backend created its own `SecureKeyProvider()` next to
    // `instance`; even then the read-then-create must run once per entry.
    final storage = _SlowStorage();
    final a = SecureKeyProvider(storage: storage);
    final b = SecureKeyProvider(storage: storage);
    final results = await Future.wait([a.getOrCreateKey(), b.getOrCreateKey()]);
    expect(results[0], isNotNull);
    expect(results[1], results[0]);
    expect(storage.writes, 1);
    expect(
      base64Decode(storage.values[SecureKeyProvider.hiveBoxKeyName]!),
      results[0],
    );
  });

  test(
    'hive key and device-pool key lookups never overlap in storage',
    () async {
      final storage = _SlowStorage();
      final provider = SecureKeyProvider(storage: storage);
      final results = await Future.wait([
        provider.getOrCreateKey(),
        provider.devicePoolKey(),
        provider.getOrCreateKey(),
        provider.devicePoolKey(),
      ]);
      expect(results.every((key) => key != null), isTrue);
      expect(results[0], isNot(results[1]), reason: '两把钥匙必须是不同的随机值');
      expect(storage.maxInFlight, 1);
      expect(storage.writes, 2);
    },
  );

  test('a read that keeps throwing keeps the real error text', () async {
    final storage = _PlatformThrowingStorage();
    final provider = SecureKeyProvider(
      storage: storage,
      errorRetryDelay: Duration.zero,
    );
    expect(await provider.getOrCreateKey(), isNull);
    // One retry, never a write: the stored entry is not replaced.
    expect(storage.reads, 2);
    expect(storage.writes, 0);
    final failure = provider.failureFor(SecureKeyProvider.hiveBoxKeyName)!;
    expect(failure.operation, 'read');
    expect(failure.errorType, 'PlatformException');
    expect(failure.describe(), contains('javax.crypto.BadPaddingException'));
    expect(failure.describe(), contains('BAD_DECRYPT'));
    // Stack frames are dropped.
    expect(failure.describe(), isNot(contains('at com.')));
  });

  test('a transient platform error is retried once and recovers', () async {
    final storage = _PlatformThrowingStorage(failFirstReads: 1)
      ..values[SecureKeyProvider.hiveBoxKeyName] =
          'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=';
    final provider = SecureKeyProvider(
      storage: storage,
      errorRetryDelay: const Duration(milliseconds: 1),
    );
    final key = await provider.getOrCreateKey();
    expect(key, isNotNull);
    expect(key!.last, 31);
    expect(storage.reads, 2);
    expect(storage.writes, 0);
    expect(provider.failureFor(SecureKeyProvider.hiveBoxKeyName), isNull);
  });

  test('a missing platform implementation is not retried', () async {
    final storage = _MissingPluginStorage();
    final provider = SecureKeyProvider(storage: storage);
    expect(await provider.getOrCreateKey(), isNull);
    expect(storage.reads, 1);
  });

  test('resetKeys deletes and regenerates only the named key', () async {
    final storage = _PlatformThrowingStorage(failFirstReads: 2);
    final provider = SecureKeyProvider(
      storage: storage,
      errorRetryDelay: Duration.zero,
    );
    storage.values[SecureKeyProvider.devicePoolKeyName] =
        'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=';
    storage.values[SecureKeyProvider.hiveBoxKeyName] = 'broken-entry';
    expect(await provider.getOrCreateKey(), isNull);

    await provider.resetKeys([SecureKeyProvider.hiveBoxKeyName]);
    final fresh = await provider.getOrCreateKey();
    expect(fresh, isNotNull);
    expect(fresh!.length, 32);
    expect(storage.deletes, 1);
    expect(
      storage.values[SecureKeyProvider.devicePoolKeyName],
      'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=',
      reason: '设备池密钥不在重置列表里，必须原样保留',
    );
    expect(provider.failureFor(SecureKeyProvider.hiveBoxKeyName), isNull);
  });
}

class _MissingPluginStorage implements SecureKeyStorage {
  int reads = 0;

  @override
  Future<String?> read(String key) async {
    reads++;
    throw MissingPluginException('No implementation found');
  }

  @override
  Future<void> write(String key, String value) async =>
      throw MissingPluginException('No implementation found');

  @override
  Future<void> delete(String key) async =>
      throw MissingPluginException('No implementation found');
}

class _FlakyStorage implements SecureKeyStorage {
  _FlakyStorage({required this.hangFirstReads});

  final int hangFirstReads;
  final Map<String, String> values = {};
  int reads = 0;
  int writes = 0;

  @override
  Future<String?> read(String key) {
    reads++;
    if (reads <= hangFirstReads) return Completer<String?>().future;
    return Future.value(values[key]);
  }

  @override
  Future<void> write(String key, String value) async {
    writes++;
    values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    values.remove(key);
  }
}

class _HangingStorage implements SecureKeyStorage {
  @override
  Future<String?> read(String key) => Completer<String?>().future;

  @override
  Future<void> write(String key, String value) => Completer<void>().future;

  @override
  Future<void> delete(String key) => Completer<void>().future;
}

class _ThrowingStorage implements SecureKeyStorage {
  @override
  Future<String?> read(String key) async => throw StateError('no keystore');

  @override
  Future<void> write(String key, String value) async =>
      throw StateError('no keystore');

  @override
  Future<void> delete(String key) async => throw StateError('no keystore');
}

class _MapStorage implements SecureKeyStorage {
  final Map<String, String> values = {};

  @override
  Future<String?> read(String key) async => values[key];

  @override
  Future<void> write(String key, String value) async {
    values[key] = value;
  }

  @override
  Future<void> delete(String key) async => values.remove(key);
}

/// Answers after a short real delay and records how many calls overlap, so
/// tests can check that the global queue keeps one call in flight.
class _SlowStorage implements SecureKeyStorage {
  final Map<String, String> values = {};
  int reads = 0;
  int writes = 0;
  int inFlight = 0;
  int maxInFlight = 0;

  Future<T> _slow<T>(T Function() body) async {
    inFlight++;
    if (inFlight > maxInFlight) maxInFlight = inFlight;
    try {
      await Future<void>.delayed(const Duration(milliseconds: 15));
      return body();
    } finally {
      inFlight--;
    }
  }

  @override
  Future<String?> read(String key) => _slow(() {
    reads++;
    return values[key];
  });

  @override
  Future<void> write(String key, String value) => _slow(() {
    writes++;
    values[key] = value;
  });

  @override
  Future<void> delete(String key) => _slow(() {
    values.remove(key);
  });
}

/// flutter_secure_storage's Android failure when the Keystore-wrapped AES key
/// no longer decrypts the stored entries: every read throws.
PlatformException badPadding() => PlatformException(
  code: 'Exception encountered',
  message: 'read',
  details:
      'javax.crypto.BadPaddingException: error:1e000065:Cipher functions:'
      'OPENSSL_internal:BAD_DECRYPT\n'
      '\tat com.android.org.conscrypt.NativeCrypto.EVP_CipherFinal_ex(Native Method)\n'
      '\tat com.it_nomads.fluttersecurestorage.ciphers.StorageCipher18Implementation.decrypt(StorageCipher18Implementation.java:96)\n',
);

class _PlatformThrowingStorage implements SecureKeyStorage {
  _PlatformThrowingStorage({this.failFirstReads = 1 << 30});

  final int failFirstReads;
  final Map<String, String> values = {};
  int reads = 0;
  int writes = 0;
  int deletes = 0;

  @override
  Future<String?> read(String key) async {
    reads++;
    if (reads <= failFirstReads) throw badPadding();
    return values[key];
  }

  @override
  Future<void> write(String key, String value) async {
    writes++;
    values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    deletes++;
    values.remove(key);
  }
}
