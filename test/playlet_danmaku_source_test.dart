import 'package:flutter_test/flutter_test.dart';
import 'package:fqapp/models/playlet_comment.dart';
import 'package:fqapp/services/playback_debug.dart';
import 'package:fqapp/widgets/player/playlet_danmaku_loader.dart';

/// 弹幕回包的真实形状（2026-11-17 线上抓取，vid 7704572446807968793 首窗；
/// 只保留解析用到的字段，评论 id / 用户 id / 推荐串换成占位值）。
Map<String, dynamic> livePayload({
  required List<Map<String, dynamic>> items,
  String source = '8662',
}) => {
  'code': 0,
  'message': 'success',
  'data': {
    'common_list_info': {
      'cursor':
          '{"start_offset_time":0,"end_offset_time":30000,'
          '"danmaku_window_count":12,"danmaku_count":384,"session_id":"S"}',
      'has_more': true,
      'total': 0,
    },
    'data_list': items,
    'extra': {'next_query_danmaku_list_time': 30000},
    'fq_danmaku_source': source,
  },
};

Map<String, dynamic> liveItem(String id, String text, int offsetMs) => {
  'data_type': 4,
  'comment': {
    'comment_id': id,
    'common': {
      'comment_type': 0,
      'content': {'text': text},
      'create_timestamp': 1763370000,
      'group_id': '7704572446807968793',
      'service_id': 20,
      'status': 0,
      'user_info': {'user_id': 'U'},
    },
    'expand': {
      'book_id': '0',
      'book_info_list': <Object>[],
      'creator_id': '0',
      'extra': {'comment_jump_schema': ''},
      'log_extra': {'recommend_info': 'R'},
      'offset_time': offsetMs,
      'relate_info_list': <Object>[],
      'reply_show_count': 0,
      'reply_show_row': 0,
      'show_boost': 0,
    },
    'stat': {'digg_count': 0, 'reply_count': 0},
    'user_action': {
      'is_author_stick': false,
      'user_digg': false,
      'user_disagree': false,
    },
  },
};

void main() {
  test('a live-shaped window parses every danmaku with its offset', () {
    final page = PlayletCommentPage.fromDanmakuPayload(
      livePayload(
        items: [
          liveItem('c1', '干净利落，一点都不含糊', 1117),
          liveItem('c2', '好看', 2127),
          liveItem('c3', '来了来了', 29187),
        ],
      ),
    );
    expect(page.comments.map((c) => c.offsetMs), [1117, 2127, 29187]);
    expect(page.comments.first.text, '干净利落，一点都不含糊');
    expect(page.hasMore, isTrue);
    expect(page.nextQueryMs, 30000);
    expect(page.serverDanmakuCount, 384);
    expect(page.danmakuSource, '8662');
  });

  test('a withheld window keeps the server count and the source tag', () {
    final page = PlayletCommentPage.fromDanmakuPayload(
      livePayload(items: const [], source: '8662-empty'),
    );
    expect(page.comments, isEmpty);
    expect(page.serverDanmakuCount, 384);
    expect(page.danmakuSource, '8662-empty');
    expect(page.cursor, contains('danmaku_count'));
  });

  test('a cursor that is not JSON counts as zero', () {
    final page = PlayletCommentPage.fromDanmakuPayload({
      'code': 0,
      'data': {
        'common_list_info': {'cursor': 'opaque'},
        'data_list': const [],
      },
    });
    expect(page.serverDanmakuCount, 0);
    expect(page.danmakuSource, '');
  });

  test('the loader tallies window sources and clears them per video', () async {
    final pages = <PlayletCommentPage>[
      PlayletCommentPage.fromDanmakuPayload(
        livePayload(items: [liveItem('o1', '海外', 500)], source: 'oversea'),
      ),
    ];
    final loader = DanmakuLoader(fetch: (_) async => pages.removeAt(0));
    loader.reset(vid: 'V1');
    await pumpEventQueue();
    expect(loader.stats.serverCount, 384);
    expect(loader.stats.windows, 1);
    expect(loader.stats.overseaWindows, 1);
    expect(loader.stats.withheldWindows, 0);

    pages.add(
      PlayletCommentPage.fromDanmakuPayload(
        livePayload(items: const [], source: '8662-empty'),
      ),
    );
    loader.reset(vid: 'V2');
    expect(loader.stats.windows, 0, reason: '切集清零');
    await pumpEventQueue();
    expect(loader.stats.windows, 1);
    expect(loader.stats.withheldWindows, 1);
    loader.dispose();
  });

  test('the overlay row separates "not wired" from "not served"', () {
    expect(playbackDanmakuText(0, const DanmakuLoadStats()), '已加载 0 条');
    expect(
      playbackDanmakuText(
        0,
        const DanmakuLoadStats(
          serverCount: 384,
          windows: 2,
          withheldWindows: 2,
        ),
      ),
      '已加载 0 条 · 服务端计数 384 · 2 窗（未下发 2）',
    );
    expect(
      playbackDanmakuText(
        5,
        const DanmakuLoadStats(
          serverCount: 384,
          windows: 3,
          overseaWindows: 1,
          withheldWindows: 1,
        ),
      ),
      '已加载 5 条 · 服务端计数 384 · 3 窗（海外库兜底 1，未下发 1）',
    );
  });
}
