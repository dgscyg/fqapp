import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import 'package:fqapp/models/media_item.dart';
import 'package:fqapp/services/chapter_cache_store.dart';
import 'package:fqapp/services/chapter_download.dart';

/// 不触磁盘的 [ChapterCache]：读永远 miss，写只记账。
class _FakeChapterCache implements ChapterCache {
  final reads = <String>[];
  final writes =
      <
        ({
          String bookId,
          String chapterId,
          String title,
          String text,
          bool pinned,
        })
      >[];
  int saveBookCalls = 0;

  @override
  int get chapterCapacity => 500;

  @override
  Future<String?> read({
    required String bookId,
    required String chapterId,
  }) async {
    reads.add(chapterId);
    return null;
  }

  @override
  Future<void> write({
    required String bookId,
    required String chapterId,
    required String title,
    required String text,
    bool pinned = false,
  }) async {
    writes.add((
      bookId: bookId,
      chapterId: chapterId,
      title: title,
      text: text,
      pinned: pinned,
    ));
  }

  @override
  Future<void> saveBook(CachedBook book) async {
    saveBookCalls++;
  }

  @override
  Future<Set<String>> cachedChapterIds(String bookId) async => {};

  @override
  Future<CachedBook?> cachedBook(String bookId) async => null;

  @override
  Future<CachedBook?> savedCatalogue(String bookId) async => null;
}

CachedBook _book(int chapterCount) => CachedBook(
  id: 'book-1',
  title: '书',
  chapters: [
    for (var i = 1; i <= chapterCount; i++)
      Chapter(itemId: 'c$i', title: '第$i章', volumeName: ''),
  ],
);

void main() {
  test('双启：运行中再次 start 被忽略，只跑一批', () async {
    final gate = Completer<void>();
    final entered = Completer<void>();
    final batchCalls = <List<String>>[];
    final cache = _FakeChapterCache();
    final download = ChapterDownload(
      cache: cache,
      book: _book(3),
      batchLoader: (chapters) async {
        batchCalls.add(chapters.map((c) => c.itemId).toList());
        // 第一批进来 = 批次已在跑；用门挡住它，好让第二次 start 有机会
        // 在运行中发生。
        if (!entered.isCompleted) entered.complete();
        await gate.future;
        return {
          for (final c in chapters) c.itemId: '正文${c.itemId}内容……',
        };
      },
    );
    addTearDown(download.dispose);

    final first = download.start(startIndex: 0, count: 3);
    await entered.future;
    expect(download.value.running, isTrue);

    // 第二次 start 在运行中：立即返回，不产生第二批请求，
    // 不重置进度、不开第二批。
    await download.start(startIndex: 0, count: 3);
    expect(batchCalls, hasLength(1));

    gate.complete();
    await first;
    expect(batchCalls, [
      ['c1', 'c2', 'c3'],
    ]);
    expect(cache.writes, hasLength(3));
    expect(download.value.running, isFalse);
    expect(download.value.completed, 3);
    expect(download.value.total, 3);
    expect(download.value.message, isNotNull);
  });

  test('取消：门开后不再落盘，已保存数保持 0', () async {
    final gate = Completer<void>();
    final entered = Completer<void>();
    final cache = _FakeChapterCache();
    final download = ChapterDownload(
      cache: cache,
      book: _book(2),
      batchLoader: (chapters) async {
        if (!entered.isCompleted) entered.complete();
        await gate.future;
        return {
          for (final c in chapters) c.itemId: '正文${c.itemId}内容……',
        };
      },
    );
    addTearDown(download.dispose);

    final first = download.start(startIndex: 0, count: 2);
    await entered.future;
    expect(download.value.running, isTrue);

    download.cancel();
    expect(download.value.running, isFalse);
    expect(download.value.completed, 0);
    expect(download.value.message, contains('已停止'));

    // 在途的 loader 回包后，批次必须认出 job 已死：不写盘、不改计数、
    // 不覆盖取消的终态。
    gate.complete();
    await first;
    expect(cache.writes, isEmpty);
    expect(download.value.completed, 0);
    expect(download.value.running, isFalse);
    expect(download.value.message, contains('已停止'));
  });
  test('分批：超过 batchSize 的章节按批请求，批间顺序执行', () async {
    final batchCalls = <List<String>>[];
    final cache = _FakeChapterCache();
    final download = ChapterDownload(
      cache: cache,
      book: _book(7),
      batchSize: 3,
      batchLoader: (chapters) async {
        batchCalls.add(chapters.map((c) => c.itemId).toList());
        return {
          for (final c in chapters) c.itemId: '正文${c.itemId}内容……',
        };
      },
    );
    addTearDown(download.dispose);

    await download.start(startIndex: 0, count: 7);
    // 7 章、每批 3：三批 3+3+1，顺序执行。
    expect(batchCalls, [
      ['c1', 'c2', 'c3'],
      ['c4', 'c5', 'c6'],
      ['c7'],
    ]);
    expect(cache.writes, hasLength(7));
    expect(download.value.completed, 7);
  });
}
