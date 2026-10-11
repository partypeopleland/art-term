package net.missart.artterm.voice

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.speech.ModelDownloadListener
import android.speech.RecognitionSupport
import android.speech.RecognitionSupportCallback
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import androidx.appcompat.app.AppCompatActivity
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.Executor

private const val DEFAULT_LANGUAGE = "zh-TW"
private const val MIC_ALIAS = "microphone"
private const val SUPPORT_QUERY_TIMEOUT_MS = 5_000L

@InvokeArg
class StartArgs {
    var sessionId: String = ""
    var language: String = ""
    var allowLanguageSwitch: Boolean = false
}

@InvokeArg
class SessionArgs {
    var sessionId: String = ""
}

@InvokeArg
class DownloadLanguageArgs {
    var language: String = ""
}

/**
 * On-device speech recognition for the mobile terminal. Single session;
 * recognized text is emitted through the `voice` event and is never
 * logged or persisted.
 */
@TauriPlugin(
    permissions = [
        Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = MIC_ALIAS),
    ],
)
class MobileVoicePlugin(private val activity: Activity) : Plugin(activity) {
    private val handler = Handler(Looper.getMainLooper())
    private val mainExecutor = Executor { command -> handler.post(command) }

    private var activeSession: VoiceSession? = null
    private var activeSessionId: String? = null
    private var downloadRecognizer: SpeechRecognizer? = null

    // ================= events =================

    private fun emit(sessionId: String, kind: String, fill: JSObject.() -> Unit) {
        val payload = JSObject()
        payload.put("sessionId", sessionId)
        payload.put("kind", kind)
        payload.fill()
        trigger("voice", payload)
    }

    private fun sinkFor(sessionId: String): VoiceSink = object : VoiceSink {
        override fun onState(state: String, languageSwitch: Boolean) {
            emit(sessionId, "state") {
                put("state", state)
                put("languageSwitch", languageSwitch)
            }
            if (state == "idle" && sessionId == activeSessionId) {
                activeSession = null
                activeSessionId = null
            }
        }

        override fun onPartial(text: String) {
            emit(sessionId, "partial") { put("text", text) }
        }

        override fun onFinal(text: String) {
            emit(sessionId, "final") { put("text", text) }
        }

        override fun onError(error: String) {
            emit(sessionId, "error") { put("error", error) }
        }
    }

    // ================= helpers =================

    private fun permissionText(): String = when (getPermissionState(MIC_ALIAS)) {
        PermissionState.GRANTED -> "granted"
        PermissionState.DENIED -> "denied"
        else -> "prompt"
    }

    /** Segmented on-device sessions require EXTRA_AUDIO_SOURCE (API 33). */
    private fun isAvailable(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            try {
                SpeechRecognizer.isOnDeviceRecognitionAvailable(activity)
            } catch (_: Exception) {
                false
            }

    private fun speechIntent(lang: String): Intent =
        Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(
                RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
            )
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE, lang)
            .putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
            .putExtra(RecognizerIntent.EXTRA_CALLING_PACKAGE, activity.packageName)

    // ================= commands =================

    @Command
    fun status(invoke: Invoke) {
        handler.post {
            val result = JSObject()
            result.put("sdkInt", Build.VERSION.SDK_INT)
            result.put("permission", permissionText())
            result.put(
                "languageSwitchSupported",
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE,
            )
            val available = isAvailable()
            result.put("available", available)
            if (!available) {
                result.put("installed", JSArray())
                result.put("supported", JSArray())
                invoke.resolve(result)
                return@post
            }
            querySupport { installed, supported ->
                val installedArr = JSArray()
                installed.forEach { installedArr.put(it) }
                val supportedArr = JSArray()
                supported.forEach { supportedArr.put(it) }
                result.put("installed", installedArr)
                result.put("supported", supportedArr)
                invoke.resolve(result)
            }
        }
    }

    @Command
    fun requestPermission(invoke: Invoke) {
        handler.post {
            if (getPermissionState(MIC_ALIAS) == PermissionState.GRANTED) {
                resolvePermission(invoke)
            } else {
                requestPermissionForAlias(MIC_ALIAS, invoke, "micPermissionResult")
            }
        }
    }

    @PermissionCallback
    private fun micPermissionResult(invoke: Invoke) {
        resolvePermission(invoke)
    }

    private fun resolvePermission(invoke: Invoke) {
        val result = JSObject()
        result.put("permission", permissionText())
        invoke.resolve(result)
    }

    @Command
    fun downloadLanguage(invoke: Invoke) {
        handler.post {
            val args = invoke.parseArgs(DownloadLanguageArgs::class.java)
            val lang = args.language.trim()
            if (lang.isEmpty() || !isAvailable()) {
                invoke.reject(if (lang.isEmpty()) "LANGUAGE_UNAVAILABLE" else "NOT_AVAILABLE")
                return@post
            }
            if (downloadRecognizer != null) {
                invoke.reject("BUSY")
                return@post
            }
            val rec = try {
                SpeechRecognizer.createOnDeviceSpeechRecognizer(activity)
            } catch (_: Exception) {
                null
            }
            if (rec == null) {
                invoke.reject("NOT_AVAILABLE")
                return@post
            }
            downloadRecognizer = rec
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                try {
                    rec.triggerModelDownload(
                        speechIntent(lang),
                        mainExecutor,
                        object : ModelDownloadListener {
                            override fun onProgress(percent: Int) {
                                emit("", "download") { put("progress", percent) }
                            }

                            override fun onSuccess() {
                                emit("", "download") {
                                    put("progress", 100)
                                    put("downloadResult", "success")
                                }
                                finishDownload(rec)
                            }

                            override fun onScheduled() {
                                // System schedules the download; no further
                                // callbacks arrive for this listener.
                                emit("", "download") {
                                    put("downloadResult", "scheduled")
                                }
                                finishDownload(rec)
                            }

                            override fun onError(error: Int) {
                                emit("", "download") {
                                    put("downloadResult", "error")
                                    put("error", downloadErrorName(error))
                                }
                                finishDownload(rec)
                            }
                        },
                    )
                    val result = JSObject()
                    result.put("started", true)
                    invoke.resolve(result)
                } catch (_: Exception) {
                    finishDownload(rec)
                    invoke.reject("NOT_AVAILABLE")
                }
            } else {
                // API 33: single-argument variant has no callbacks; the system
                // decides when the model lands. Report it as scheduled.
                try {
                    @Suppress("DEPRECATION")
                    rec.triggerModelDownload(speechIntent(lang))
                    emit("", "download") { put("downloadResult", "scheduled") }
                    val result = JSObject()
                    result.put("started", true)
                    invoke.resolve(result)
                } catch (_: Exception) {
                    invoke.reject("NOT_AVAILABLE")
                }
                handler.postDelayed({ finishDownload(rec) }, 2_000)
            }
        }
    }

    private fun finishDownload(rec: SpeechRecognizer) {
        if (downloadRecognizer === rec) downloadRecognizer = null
        try {
            rec.destroy()
        } catch (_: Exception) {
        }
    }

    private fun downloadErrorName(code: Int): String = when (code) {
        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED,
        SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE,
        -> "LANGUAGE_UNAVAILABLE"
        else -> "ERROR_CODE_$code"
    }

    @Command
    fun start(invoke: Invoke) {
        handler.post {
            val args = invoke.parseArgs(StartArgs::class.java)
            if (args.sessionId.isBlank()) {
                invoke.reject("INVALID_ARGS")
                return@post
            }
            if (getPermissionState(MIC_ALIAS) != PermissionState.GRANTED) {
                invoke.reject("PERMISSION_DENIED")
                return@post
            }
            if (!isAvailable()) {
                invoke.reject("NOT_AVAILABLE")
                return@post
            }
            // Single session: a new start cancels the previous one.
            activeSession?.cancel()
            activeSession = null
            activeSessionId = null
            val session = VoiceSession(
                activity,
                args.language.ifBlank { DEFAULT_LANGUAGE },
                args.allowLanguageSwitch,
                sinkFor(args.sessionId),
            )
            activeSession = session
            activeSessionId = args.sessionId
            session.start()
            val failed = session.terminalError
            if (failed != null) {
                invoke.reject(failed)
                return@post
            }
            val result = JSObject()
            result.put("ok", true)
            result.put("mechanism", "segmented-audio-source")
            invoke.resolve(result)
        }
    }

    @Command
    fun stop(invoke: Invoke) {
        handler.post {
            val args = invoke.parseArgs(SessionArgs::class.java)
            val session = activeSession
            val result = JSObject()
            if (session != null && activeSessionId == args.sessionId) {
                session.stop()
                result.put("ok", true)
            } else {
                result.put("ok", false)
            }
            invoke.resolve(result)
        }
    }

    @Command
    fun cancel(invoke: Invoke) {
        handler.post {
            val args = invoke.parseArgs(SessionArgs::class.java)
            val session = activeSession
            val result = JSObject()
            if (session != null && activeSessionId == args.sessionId) {
                session.cancel()
                result.put("ok", true)
            } else {
                result.put("ok", false)
            }
            invoke.resolve(result)
        }
    }

    // ================= lifecycle =================

    override fun onStop(activity: AppCompatActivity) {
        // The recorder start gate must be sealed before onStop returns:
        // posting the whole cancel would leave a window in which a
        // recorder worker that is scheduled but has not run yet still
        // sees recording && !finished and opens the microphone.
        // sealForBackground() marks the session finished and stops a
        // live AudioRecord synchronously under recorderLock; the
        // recognizer and event teardown still run on the main thread.
        val session = activeSession
        if (session != null && session.sealForBackground()) {
            handler.post {
                if (activeSession === session) {
                    session.cancelBackground()
                }
            }
        }
    }

    override fun onDestroy(activity: AppCompatActivity) {
        activeSession?.cancel()
        activeSession = null
        activeSessionId = null
        downloadRecognizer?.destroy()
        downloadRecognizer = null
    }

    // ================= language support query =================

    /** Fresh recognizer per query, destroyed on completion or timeout. */
    private fun querySupport(done: (List<String>, List<String>) -> Unit) {
        val rec = try {
            SpeechRecognizer.createOnDeviceSpeechRecognizer(activity)
        } catch (_: Exception) {
            null
        }
        if (rec == null) {
            done(emptyList(), emptyList())
            return
        }
        var resolved = false
        val timeout = Runnable {
            if (!resolved) {
                resolved = true
                try {
                    rec.destroy()
                } catch (_: Exception) {
                }
                done(emptyList(), emptyList())
            }
        }
        val finish = { installed: List<String>, supported: List<String> ->
            if (!resolved) {
                resolved = true
                handler.removeCallbacks(timeout)
                try {
                    rec.destroy()
                } catch (_: Exception) {
                }
                done(installed, supported)
            }
        }
        handler.postDelayed(timeout, SUPPORT_QUERY_TIMEOUT_MS)
        try {
            rec.checkRecognitionSupport(
                speechIntent(DEFAULT_LANGUAGE),
                mainExecutor,
                object : RecognitionSupportCallback {
                    override fun onSupportResult(support: RecognitionSupport) {
                        finish(
                            support.installedOnDeviceLanguages.toList(),
                            support.supportedOnDeviceLanguages.toList(),
                        )
                    }

                    override fun onError(error: Int) {
                        finish(emptyList(), emptyList())
                    }
                },
            )
        } catch (_: Exception) {
            finish(emptyList(), emptyList())
        }
    }
}
