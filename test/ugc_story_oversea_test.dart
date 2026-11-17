import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:fqapp/services/api_client.dart';
import 'package:fqapp/services/backend_transport.dart';

/// Serves one canned body for every request and records the URLs.
class _FixtureTransport implements BackendTransport {
  _FixtureTransport(this.body);

  final Map<String, dynamic> body;
  final List<Uri> urls = [];

  @override
  String get baseUrl => 'http://localhost:9000';

  @override
  String get capability => '';

  @override
  Future<BackendResponse> send(
    String method,
    Uri url, {
    Uint8List? body,
    Duration? timeout,
    BackendRequest? request,
  }) async {
    urls.add(url);
    return BackendResponse(
      200,
      Uint8List.fromList(utf8.encode(jsonEncode(this.body))),
      'application/json',
    );
  }

  @override
  Future<void> close() async {}
}

/// Recorded from the international feed (aid 1967 / 68132) on 2026-11-17;
/// the same file drives the Rust cursor test (rust/testdata).
Map<String, dynamic> _fixture() =>
    jsonDecode(File('test/fixtures/ugc_feed_oversea.json').readAsStringSync())
        as Map<String, dynamic>;

void main() {
  test('the international short-story feed parses into cards', () async {
    final api = ApiClient(transport: _FixtureTransport(_fixture()));
    final page = await api.ugcStories();

    expect(page.items, hasLength(3));
    final first = page.items.first;
    expect(first.postId, '7632880745031275289');
    expect(first.relatedBookId, '7632881315112569881');
    expect(first.relatedChapterId, '7632880373394850329');
    expect(first.title, isNotEmpty);
    expect(first.cover, startsWith('https://'));
    expect(first.summary, isNotEmpty);
    expect(page.sessionId, (_fixture()['data'] as Map)['session_id']);
    expect(page.sessionId, isNotEmpty);
    expect(page.hasMore, isTrue);
    // Raw international shape: the cursor does not advance by itself. This is
    // what made the 短篇 tab fail with 「内容解析失败」 before the core started
    // rewriting it (rust/src/endpoints/home.rs `normalize_ugc_cursor`).
    expect(page.nextOffset, 0);
  });

  test('the core-normalized cursor advances and keeps the session', () async {
    // What the core returns for offset 0 since 1.0.214: next_offset advanced
    // by the delivered cards, session untouched.
    final body = _fixture();
    (body['data'] as Map<String, dynamic>)['next_offset'] = 3;
    final transport = _FixtureTransport(body);
    final api = ApiClient(transport: transport);

    final first = await api.ugcStories();
    expect(first.hasMore, isTrue);
    expect(first.nextOffset, greaterThan(0));

    await api.ugcStories(offset: first.nextOffset!, sessionId: first.sessionId);
    final next = transport.urls.last;
    expect(next.path, '/api/v1/recommend/ugc-stories');
    expect(next.queryParameters['offset'], '3');
    expect(next.queryParameters['session_id'], first.sessionId);
  });
}
