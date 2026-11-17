import 'dart:async';

import 'package:flutter/foundation.dart';

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
  Future<void> delete(String key) async {
    values.remove(key);
  }
}
