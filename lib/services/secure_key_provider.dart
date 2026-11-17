import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:hive_ce/hive.dart';

/// Minimal read/write/delete surface over the platform secure store, kept as
/// an interface so tests can substitute an in-memory fake instead of driving
/// a MethodChannel.
abstract interface class SecureKeyStorage {
  Future<String?> read(String key);
  Future<void> write(String key, String value);
  Future<void> delete(String key);
}

/// Production backend: `flutter_secure_storage` (Android Keystore-wrapped
/// AES in SharedPreferences, iOS Keychain). The raw key bytes never touch
/// SharedPreferences or any other plaintext store.
class FlutterSecureKeyStorage implements SecureKeyStorage {
  const FlutterSecureKeyStorage([this._storage = const FlutterSecureStorage()]);

  final FlutterSecureStorage _storage;

  @override
  Future<String?> read(String key) => _storage.read(key: key);

  @override
  Future<void> write(String key, String value) =>
      _storage.write(key: key, value: value);

  @override
  Future<void> delete(String key) => _storage.delete(key: key);
}

/// In-memory fake for tests and for environments without a platform channel.
class InMemorySecureKeyStorage implements SecureKeyStorage {
  final Map<String, String> _values = {};

  @override
  Future<String?> read(String key) async => _values[key];

  @override
  Future<void> write(String key, String value) async {
    _values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    _values.remove(key);
  }
}

/// Why a key could not be loaded: which step failed and the original error's
/// type and message. Never carries key material — only exception text from
/// the platform (e.g. `javax.crypto.BadPaddingException: ...`).
@immutable
class SecureKeyFailure {
  const SecureKeyFailure({
    required this.keyName,
    required this.operation,
    required this.errorType,
    required this.message,
  });

  /// Secure-storage entry name ([SecureKeyProvider.hiveBoxKeyName] or
  /// [SecureKeyProvider.devicePoolKeyName]).
  final String keyName;

  /// `read`, `write` or `delete`.
  final String operation;

  /// Runtime type of the original error (`PlatformException`,
  /// `TimeoutException`, ...).
  final String errorType;

  /// Sanitized one-line message of the original error.
  final String message;

  bool get isTimeout => errorType == 'TimeoutException';

  /// Short form for UI / logs: `read PlatformException: ...`.
  String describe() => message.isEmpty
      ? '$operation $errorType'
      : '$operation $errorType: $message';

  @override
  String toString() => 'SecureKeyFailure($keyName, ${describe()})';

  static SecureKeyFailure from(
    String keyName,
    String operation,
    Object error,
  ) => SecureKeyFailure(
    keyName: keyName,
    operation: operation,
    errorType: error.runtimeType.toString(),
    message: sanitizeError(error),
  );

  /// One-line description of [error] without stack traces. For a
  /// `PlatformException` from flutter_secure_storage the Java exception lives
  /// in `details` as a full stack trace; only its first line and the last
  /// `Caused by:` line are kept (they name the exception and its cause).
  @visibleForTesting
  static String sanitizeError(Object error) {
    String text;
    if (error is PlatformException) {
      final parts = <String>[
        if (error.code.isNotEmpty) '[${error.code}]',
        if ((error.message ?? '').isNotEmpty) error.message!,
      ];
      final details = error.details;
      if (details is String && details.trim().isNotEmpty) {
        final lines = details
            .split('\n')
            .map((line) => line.trim())
            .where((line) => line.isNotEmpty)
            .toList();
        if (lines.isNotEmpty) parts.add(lines.first);
        final cause = lines.lastWhere(
          (line) => line.startsWith('Caused by:'),
          orElse: () => '',
        );
        if (cause.isNotEmpty && cause != lines.first) parts.add(cause);
      } else if (details != null) {
        parts.add('$details');
      }
      text = parts.join(' ');
    } else if (error is TimeoutException) {
      text =
          '超时${error.duration == null ? '' : ' ${error.duration!.inSeconds}s'}';
    } else {
      text = error.toString();
      final type = error.runtimeType.toString();
      if (text.startsWith('$type: ')) text = text.substring(type.length + 2);
      if (text.startsWith('Bad state: ')) text = text.substring(11);
    }
    text = text.replaceAll(RegExp(r'\s+'), ' ').trim();
    if (text.length > 300) text = '${text.substring(0, 300)}…';
    return text;
  }
}

/// Thrown by `EncryptedHive` when the box key cannot be obtained. A
/// [StateError] so existing "fail closed" handling keeps working; carries the
/// underlying [failure] so the bootstrap can show the real cause and offer
/// the destructive reset.
class SecureKeyUnavailableError extends StateError {
  SecureKeyUnavailableError(super.message, {this.failure});

  final SecureKeyFailure? failure;
}

/// Serializes every secure-storage call in the process.
///
/// flutter_secure_storage's Android side runs calls on one worker thread, but
/// each call re-runs `ensureInitialized()` and the cipher bootstrap; issuing
/// the Hive-key and device-pool-key lookups concurrently from two providers
/// at cold start interleaved those with Keystore work. One global queue keeps
/// exactly one call in flight. Each queued call is bounded by its own
/// timeout, so a wedged call releases the queue instead of blocking it.
class _SecureStorageLock {
  static Future<void> _tail = Future<void>.value();

  static Future<T> run<T>(Future<T> Function() action) {
    final previous = _tail;
    final done = Completer<void>();
    _tail = done.future;
    return previous.then((_) => action()).whenComplete(done.complete);
  }
}

/// Owns the random 256-bit keys that encrypt local data at rest.
///
/// One key per purpose, all base64-encoded in secure storage:
/// - [hiveBoxKeyName]: the Hive `HiveAesCipher` key for every encrypted box
///   (see `EncryptedHive`);
/// - [devicePoolKeyName]: the Rust device-pool file key, handed to the core
///   via the `set_device_pool_key` FRB call at startup (P1-2). Sharing the
///   infrastructure but not the key material: compromising one store must not
///   open the other.
///
/// Use [instance] everywhere in the app: one provider means one cache and one
/// in-flight load per key, and every platform call goes through one global
/// queue (see `_SecureStorageLock`).
///
/// A missing entry is created once (random 256-bit via
/// `Hive.generateSecureKey`) and persisted; a present-but-malformed entry is
/// treated as a lost key and replaced, which makes the boxes encrypted under
/// the old key unreadable — `EncryptedHive` then treats those boxes as empty
/// rather than falling back to plaintext. When secure storage itself throws
/// (no platform channel, Keystore failure), `null` is returned, the original
/// error is kept in [failureFor], and callers degrade per their own policy.
class SecureKeyProvider {
  SecureKeyProvider({
    SecureKeyStorage? storage,
    this.storageTimeout = const Duration(seconds: 10),
    this.errorRetryDelay = const Duration(milliseconds: 400),
  }) : _storage = storage ?? const FlutterSecureKeyStorage();

  static final SecureKeyProvider instance = SecureKeyProvider();

  /// Hive box encryption key (base64 of 32 random bytes).
  static const hiveBoxKeyName = 'fqapp_hive_encryption_key_v1';

  /// Rust device-pool file encryption key (base64 of 32 random bytes).
  static const devicePoolKeyName = 'fqapp_device_pool_key_v1';

  /// How long a single secure-storage call may take before it is treated as
  /// unavailable. Normally milliseconds; the first call of a cold start also
  /// bootstraps the Keystore RSA/AES wrapping (seconds on slow devices), so
  /// this stays generous. Without a bound a wedged Keystore — or, in widget
  /// tests, an unmocked channel whose Future never resolves — would hang
  /// startup forever.
  final Duration storageTimeout;

  /// Pause before the single retry of a read that threw. Covers a transient
  /// Keystore error on cold start (the plugin re-runs its cipher setup on the
  /// next call) without turning a permanent failure into a long wait.
  final Duration errorRetryDelay;

  /// Cooldown after a failed key load: lookups within one startup return the
  /// cached failure instead of each waiting out the full storage timeout.
  /// A later retry (e.g. the bootstrap retry button after the cooldown)
  /// attempts the platform channel again.
  static const _failureCooldown = Duration(seconds: 30);

  final SecureKeyStorage _storage;
  final Map<String, Uint8List> _cache = {};

  /// In-flight loads, shared by every provider over the same storage
  /// object: a read-then-create must never run twice for one entry, or two
  /// first-launch callers would each persist a different random key and one
  /// of them would encrypt data under a key that is then overwritten. The
  /// production storage is a `const` object, so even a stray
  /// `SecureKeyProvider()` joins the same load as [instance].
  static final Expando<Map<String, Future<_KeyLoad>>> _inflightByStorage =
      Expando('secure-key-inflight');

  Map<String, Future<_KeyLoad>> get _pending =>
      _inflightByStorage[_storage] ??= {};

  final Map<String, DateTime> _failedAt = {};
  final Map<String, SecureKeyFailure> _failures = {};

  /// The last load failure for [name] (type + message of the original error),
  /// or null when the last load succeeded or none was attempted.
  SecureKeyFailure? failureFor(String name) => _failures[name];

  Future<Uint8List?> getOrCreateKey({String name = hiveBoxKeyName}) {
    final cached = _cache[name];
    if (cached != null) return Future.value(cached);
    final failedAt = _failedAt[name];
    if (failedAt != null &&
        DateTime.now().difference(failedAt) < _failureCooldown) {
      return Future.value(null);
    }
    var load = _pending[name];
    if (load == null) {
      late final Future<_KeyLoad> started;
      started = _loadOrCreate(name).whenComplete(() {
        if (identical(_pending[name], started)) _pending.remove(name);
      });
      _pending[name] = load = started;
    }
    // Every caller — including one that joined another provider's load —
    // applies the outcome to its own cache / failure record.
    return load.then((result) {
      final key = result.key;
      if (key != null) {
        _remember(name, key);
      } else {
        _failures[name] = result.failure!;
        _failedAt[name] = DateTime.now();
      }
      return key;
    });
  }

  /// One secure-storage call: queued behind every other call in the process
  /// and bounded by [storageTimeout] (the bound is inside the queue slot, so
  /// time spent waiting for the queue does not count against it).
  Future<T> _call<T>(Future<T> Function() action) =>
      _SecureStorageLock.run(() => action().timeout(storageTimeout));

  Future<_KeyLoad> _loadOrCreate(String name) async {
    var operation = 'read';
    try {
      final stored = await _readWithRetry(name);
      if (stored != null) {
        try {
          final decoded = base64Decode(stored);
          if (decoded.length == 32) {
            return _KeyLoad(Uint8List.fromList(decoded));
          }
        } on FormatException {
          // Malformed entry: fall through and replace it (key lost).
        }
      }
      operation = 'write';
      final fresh = Uint8List.fromList(Hive.generateSecureKey());
      await _call(() => _storage.write(name, base64Encode(fresh)));
      return _KeyLoad(fresh);
    } catch (error) {
      // Secure storage unavailable (or wedged past the timeout): keep the
      // original error for diagnostics; callers record the failure so the
      // rest of this startup degrades immediately instead of each caller
      // waiting out another timeout. The generated-but-unpersisted key is
      // deliberately NOT returned: persisting it is what makes it usable
      // next launch.
      return _KeyLoad(null, SecureKeyFailure.from(name, operation, error));
    }
  }

  void _remember(String name, Uint8List key) {
    _cache[name] = key;
    _failures.remove(name);
    _failedAt.remove(name);
  }

  /// Reads [name] with one retry. A Keystore that is merely slow on a cold
  /// start (timeout) or hit a transient error (the plugin rebuilds its cipher
  /// on the next call) should not turn into "key unavailable" — which makes
  /// the encrypted boxes fail closed and blocks startup — on a single missed
  /// answer. `MissingPluginException` (no platform implementation, e.g.
  /// tests) is permanent and not retried. The retry never deletes or
  /// rewrites anything.
  Future<String?> _readWithRetry(String name) async {
    try {
      return await _call(() => _storage.read(name));
    } on MissingPluginException {
      rethrow;
    } on TimeoutException {
      return await _call(() => _storage.read(name));
    } catch (_) {
      if (errorRetryDelay > Duration.zero) {
        await Future<void>.delayed(errorRetryDelay);
      }
      return await _call(() => _storage.read(name));
    }
  }

  /// Forgets recorded key-load failures so the next lookup asks secure
  /// storage again. Called by the bootstrap's manual retry: a user tapping
  /// 「重试」 must get a real attempt, not the cached failure from the 30 s
  /// cooldown.
  void clearFailureCooldown() => _failedAt.clear();

  /// Destructive recovery for a key that cannot be read back: deletes the
  /// entries in [names] from secure storage and generates fresh keys for
  /// them. Data encrypted under the old keys becomes unreadable — callers
  /// must remove it (see `LocalDataReset`). Keys not in [names] are left
  /// alone. Throws the original platform error when the delete or the new
  /// write fails, so the caller can show it.
  Future<void> resetKeys(Iterable<String> names) async {
    final targets = names.toSet();
    for (final name in targets) {
      final pending = _pending[name];
      if (pending != null) await pending;
      _cache.remove(name);
      _failedAt.remove(name);
      _failures.remove(name);
      try {
        await _call(() => _storage.delete(name));
      } catch (error) {
        _failures[name] = SecureKeyFailure.from(name, 'delete', error);
        rethrow;
      }
    }
    for (final name in targets) {
      final key = await getOrCreateKey(name: name);
      if (key == null) {
        final failure = _failures[name];
        throw SecureKeyUnavailableError(
          '重置后仍无法写入新密钥'
          '${failure == null ? '' : '（${failure.describe()}）'}',
          failure: failure,
        );
      }
    }
  }

  /// Device-pool file key for the P1-2 FRB handoff. Same infrastructure as
  /// the Hive key, separate key material.
  Future<Uint8List?> devicePoolKey() => getOrCreateKey(name: devicePoolKeyName);

  @visibleForTesting
  void resetForTest() {
    _cache.clear();
    _pending.clear();
    _failedAt.clear();
    _failures.clear();
  }
}

class _KeyLoad {
  const _KeyLoad(this.key, [this.failure]);

  final Uint8List? key;
  final SecureKeyFailure? failure;
}
