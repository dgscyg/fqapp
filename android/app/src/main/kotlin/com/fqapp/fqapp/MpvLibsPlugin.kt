package com.fqapp.fqapp

import android.content.Context
import android.util.Log
import com.alexmercerind.mediakitandroidhelper.MediaKitAndroidHelper
import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel

/**
 * Loads the prebuilt VVC-capable libmpv on demand for the media_kit short-drama
 * player (what media_kit_libs_android_video does at startup, but lazily, so
 * H.265/ExoPlayer users never map the ~35 MiB of FFmpeg libraries).
 *
 * `prepare` returns false when the libraries are not packaged or fail to load;
 * Dart then keeps ExoPlayer and H.265.
 */
class MpvLibsPlugin : FlutterPlugin, MethodChannel.MethodCallHandler {
    private var channel: MethodChannel? = null
    private var context: Context? = null

    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        context = binding.applicationContext
        channel = MethodChannel(binding.binaryMessenger, "fqapp/mpv_libs").also {
            it.setMethodCallHandler(this)
        }
    }

    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        channel?.setMethodCallHandler(null)
        channel = null
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        when (call.method) {
            "prepare" -> result.success(prepare())
            else -> result.notImplemented()
        }
    }

    private fun prepare(): Boolean = synchronized(MpvLibsPlugin::class.java) {
        loaded?.let { return it }
        val ok = try {
            System.loadLibrary("mpv")
            context?.let { MediaKitAndroidHelper.setApplicationContextJava(it) }
            true
        } catch (error: Throwable) {
            Log.w("fqapp-mpv", "VVC libmpv unavailable: ${error.javaClass.simpleName}")
            false
        }
        loaded = ok
        ok
    }

    companion object {
        @Volatile private var loaded: Boolean? = null
    }
}
