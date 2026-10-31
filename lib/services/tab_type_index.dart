import 'package:flutter/foundation.dart';

/// 服务端下发的「标题 → tab_type」索引。
///
/// 上游每个响应用自己的 tab 表说明每类的身份：书城首页的
/// `data.tab_item[].tab_type`（推荐 2 / 小说 25 / 经典 11 / 知识 12 /
/// 听书 5 / 看剧 8 / 视频 16 / 短篇 103 / 漫画 9 / 商城 15），搜索的
/// `search_tabs[].tab_type`（综合 1 / 社区 4 / 短剧 11 / 漫剧 19 /
/// 漫画 8 / 听书 2 / 书籍 3 / 买书 13 / 全文 5 / 用户 6）。
///
/// 这两套编号**互不相同**（听书在书城是 5、在搜索是 2；漫画是 9 / 8），
/// 而且会随服务端调整。客户端把编号写死就会出现「漫剧按 11 去搜，拿到
/// 的是短剧桶」这类串台，所以一律按标题从响应里取编号，只用本地表
/// 作为**首次请求之前的**兜底。
class TabTypeIndex {
  TabTypeIndex._();

  static final TabTypeIndex instance = TabTypeIndex._();

  /// 书城（`tab_item`）的 title → tab_type。
  final ValueNotifier<Map<String, int>> bookmall =
      ValueNotifier<Map<String, int>>(const {});

  /// 搜索分区（`search_tabs`）的 title → tab_type。
  final ValueNotifier<Map<String, int>> search =
      ValueNotifier<Map<String, int>>(const {});

  int? bookmallType(String title) => bookmall.value[title];

  int? searchType(String title) => search.value[title];

  /// 收下一次 `data.tab_item`。空表忽略，不覆盖已有的可用索引。
  void registerBookmall(Iterable<Map<String, dynamic>> tabs) {
    final next = <String, int>{...bookmall.value};
    var changed = false;
    for (final tab in tabs) {
      final title = tab['title']?.toString() ?? '';
      final type = _asInt(tab['tab_type']);
      if (title.isEmpty || type == null) continue;
      if (next[title] != type) {
        next[title] = type;
        changed = true;
      }
    }
    if (changed) bookmall.value = next;
  }

  /// 收下一次 `search_tabs`。搜索的桶表每次搜索都会回来，是最新的一份。
  void registerSearch(Iterable<Map<String, dynamic>> tabs) {
    final next = <String, int>{};
    for (final tab in tabs) {
      final title = tab['title']?.toString() ?? '';
      final type = _asInt(tab['tab_type']);
      if (title.isEmpty || type == null) continue;
      next[title] = type;
    }
    if (next.isNotEmpty && !mapEquals(next, search.value)) {
      search.value = next;
    }
  }

  @visibleForTesting
  void resetForTest() {
    bookmall.value = const {};
    search.value = const {};
  }

  static int? _asInt(Object? value) {
    if (value is int) return value;
    if (value is num) return value.toInt();
    if (value is String) return int.tryParse(value);
    return null;
  }
}