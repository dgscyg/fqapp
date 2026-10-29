import 'package:shared_preferences/shared_preferences.dart';

/// 章节批量下载的每批章节数（用户可配置，10–50，默认 20）。
///
/// 上游 batch_full 的官方安全上限是 30 章/批（参考实现
/// fqnovel-gonidbg 的 MaxBatchChapters），但配置上限放宽到 50：
/// 服务端对超限批次的处理是拒绝或截断，由调用方按返回结果处理，
/// 配置值本身在写入时钳制到 [min]–[max]。
class ChapterBatchPreferences {
  static const _batchSizeKey = 'chapter_batch_size';
  static const int min = 10;
  static const int max = 50;
  static const int defaultSize = 20;

  /// 读取配置的批量章节数；未配置或越界时回默认值。
  static Future<int> loadBatchSize() async {
    try {
      final preferences = await SharedPreferences.getInstance();
      final value = preferences.getInt(_batchSizeKey);
      if (value == null) return defaultSize;
      if (value < min || value > max) return defaultSize;
      return value;
    } catch (_) {
      return defaultSize;
    }
  }

  /// 写入配置（钳制到 10–50）。
  static Future<void> setBatchSize(int value) async {
    final clamped = value.clamp(min, max);
    try {
      final preferences = await SharedPreferences.getInstance();
      await preferences.setInt(_batchSizeKey, clamped);
    } catch (_) {
      // 持久化失败时本次会话仍使用内存值（调用方自行持有）。
    }
  }
}