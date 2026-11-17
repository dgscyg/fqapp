import 'dart:typed_data';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:fqapp/services/device_pool_key_retry.dart';

void main() {
  final key = Uint8List.fromList(List.filled(32, 7));

  test('a key that loads on a later attempt is handed to the core once', () {
    fakeAsync((async) {
      var loads = 0;
      final provisioned = <Uint8List>[];
      final retry = DevicePoolKeyRetry(
        loadKey: () async => ++loads < 3 ? null : key,
        provision: (k) async => provisioned.add(k),
        delays: const [
          Duration(seconds: 31),
          Duration(minutes: 2),
          Duration(minutes: 5),
          Duration(minutes: 15),
        ],
      );
      bool? delivered;
      retry.start().then((v) => delivered = v);
      // Joining a running loop does not start a second one.
      retry.start();

      async.elapse(const Duration(seconds: 30));
      expect(loads, 0, reason: 'the first attempt waits out the 30 s cooldown');
      async.elapse(const Duration(seconds: 1));
      expect(loads, 1);
      async.elapse(const Duration(minutes: 2));
      expect(loads, 2);
      expect(provisioned, isEmpty);
      async.elapse(const Duration(minutes: 5));
      expect(loads, 3);
      expect(provisioned, [key]);
      expect(delivered, isTrue);
      async.elapse(const Duration(hours: 1));
      expect(loads, 3, reason: 'no attempts after the key was delivered');
    });
  });

  test('errors are retried and the loop gives up without provisioning', () {
    fakeAsync((async) {
      var loads = 0;
      var provisions = 0;
      final retry = DevicePoolKeyRetry(
        loadKey: () async {
          loads++;
          throw StateError('keystore');
        },
        provision: (_) async => provisions++,
        delays: const [Duration(seconds: 1), Duration(seconds: 1)],
      );
      bool? delivered;
      retry.start().then((v) => delivered = v);
      async.elapse(const Duration(seconds: 5));
      expect(loads, 2);
      expect(provisions, 0);
      expect(delivered, isFalse);
    });
  });
}
