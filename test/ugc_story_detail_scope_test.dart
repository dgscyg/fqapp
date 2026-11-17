import 'package:flutter_test/flutter_test.dart';
import 'package:fqapp/models/ugc_story.dart';

Map<String, dynamic> _detail(Map<String, dynamic> extra) => {
  'code': 0,
  'data': {
    'post_id': '7642010875318276926',
    'title': '年终奖到账1块钱',
    'relate_book_id': '7642010383510686744',
    'relate_item_id': '7641949640866153022',
    'status': 1,
    'ugc_privacy': 0,
    'html_content':
        '<p idx="0"><blk>正文</blk></p><p idx="1"><blk>（全文完）</blk></p>',
    'total_word_num': 14331,
    ...extra,
  },
};

void main() {
  test('国际版详情无 truncate_flag：truncate_word_num 是段落数，不算试读', () {
    // 实测国际版详情：没有 truncate_flag，truncate_word_num（904）恰好是
    // total_word_num 与 html 正文字数之差（段落换行计数），正文是全文。
    final detail = UgcStoryDetail.fromPayload(
      _detail({'truncate_word_num': 904}),
    );
    expect(detail.isTruncated, isFalse);
    expect(detail.truncatedWordCount, isNull);
    expect(detail.availability, UgcStoryAvailability.available);
  });

  test('显式 truncate_flag 才按试读处理', () {
    final detail = UgcStoryDetail.fromPayload(
      _detail({'truncate_flag': 3, 'truncate_word_num': 14131}),
    );
    expect(detail.isTruncated, isTrue);
    expect(detail.truncatedWordCount, 14131);
  });
}
