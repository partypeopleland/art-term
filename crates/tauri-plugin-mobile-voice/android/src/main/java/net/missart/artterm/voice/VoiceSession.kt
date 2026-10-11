package net.missart.artterm.voice

import android.content.Context
import android.content.Intent
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import java.io.FileDescriptor

// OsConstants does not expose the fcntl commands needed for the pipe flags.
private const val F_GETFL = 3
private const val F_SETFL = 4
private const val O_NONBLOCK = 0x800
// After the user stops, how long to wait for the final result.
private const val STOP_TIMEOUT_MS = 15_000L
// Upper bound on chained recognizer restarts inside one session.
private const val MAX_RESTARTS = 200
// A recognizer that ends within this window counts as an immediate failure.
private const val IMMEDIATE_FAIL_MS = 1_500L
private const val MAX_IMMEDIATE_FAILS = 3
// 16kHz mono PCM16; AudioRecord reads ~50ms (3200B) per chunk.
private const val SAMPLE_RATE = 16000
private const val CHUNK_BYTES = 3200
// Queue cap (~5 minutes) so a service that never reads cannot grow memory.
private const val QUEUE_MAX_BYTES = 16000 * 2 * 300
// Silence tail after stop so endpoint detection fires before EOF.
private const val STOP_TAIL_MS = 1_000L
// Delay before chaining into a new recognizer; audio keeps buffering.
private const val CHAIN_DELAY_MS = 300L

private val SEGMENT_END_PUNCT = setOf(
    '，', '。', '！', '？', '、', '；', '：', '…',
    ',', '.', '!', '?', ';', ':',
)

/**
 * Join recognized segments with a fullwidth comma, mirroring the validated
 * POC rule: skip empty segments, do not duplicate a trailing punctuation
 * mark, and never append a final period.
 */
internal fun joinSegmentTexts(segments: List<String>): String {
    val sb = StringBuilder()
    for (seg in segments) {
        if (seg.isEmpty()) continue
        val last = sb.lastOrNull { !it.isWhitespace() }
        if (last != null && last !in SEGMENT_END_PUNCT) {
            sb.append('，')
        }
        sb.append(seg)
    }
    return sb.toString()
}

/** Event sink implemented by the plugin; every callback is a `voice` event. */
internal interface VoiceSink {
    fun onState(state: String, languageSwitch: Boolean)
    fun onPartial(text: String)
    fun onFinal(text: String)
    fun onError(error: String)
}

/**
 * One microphone recording session, ported from the Pixel-9-validated POC:
 * AudioRecord 16kHz mono PCM16 -> in-memory queue -> O_NONBLOCK pipe ->
 * createOnDeviceSpeechRecognizer + EXTRA_AUDIO_SOURCE with
 * EXTRA_SEGMENTED_SESSION = EXTRA_AUDIO_SOURCE. Pauses become segments
 * joined by fullwidth commas; a service that ends the session anyway is
 * chained to a new pipe+recognizer with buffered audio replayed; stale
 * callbacks are dropped by generation/reference checks. Audio never
 * touches disk and recognized text is never logged.
 *
 * All public methods must be called on the main thread, except
 * [sealForBackground], which is safe to call from any thread.
 */
internal class VoiceSession(
    private val context: Context,
    private val language: String,
    allowLanguageSwitch: Boolean,
    private val sink: VoiceSink,
) {
    private val handler = Handler(Looper.getMainLooper())

    // EXTRA_ENABLE_LANGUAGE_SWITCH / EXTRA_LANGUAGE_SWITCH_ALLOWED_LANGUAGES
    // were added in API 34.
    private val languageSwitch =
        allowLanguageSwitch && Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE

    private var recognizer: SpeechRecognizer? = null
    @Volatile private var finished = false
    private var userStop = false
    private var recStartTs = 0L
    private var segments = 0
    private var restarts = 0
    private var segmentedExtra = true
    private var segmentedSeen = false
    private val segmentTexts = mutableListOf<String>()
    private var lastPartial = ""
    private var immediateFails = 0
    private var gen = 0
    @Volatile private var runGen = 0
    private var timeout: Runnable? = null
    private var chain: Runnable? = null

    // ---- AudioRecord -> queue -> feeder -> pipe ----
    @Volatile private var audioRecord: AudioRecord? = null
    private var recorderThread: Thread? = null
    private var feederThread: Thread? = null
    private val pipeLock = Object()
    private val pcmQueue = ArrayDeque<ByteArray>()
    private var queueBytes = 0
    private var pipe: SessionPipe? = null // guarded by pipeLock
    @Volatile private var recording = false
    @Volatile private var draining = false
    // Serializes the recorder worker's start gate and exit mark against the
    // recording/finished writes in stop()/teardown()/finishRun() and
    // sealForBackground(). With the flag writes under the same lock, a
    // worker scheduled after a cancel never calls startRecording(), and
    // the silence tail is queued only once the worker can no longer
    // enqueue PCM.
    private val recorderLock = Object()
    private var recorderExited = true // guarded by recorderLock
    private var silenceEnqueued = false // guarded by recorderLock

    private class SessionPipe {
        var readPfd: ParcelFileDescriptor? = null
        var writePfd: ParcelFileDescriptor? = null
        @Volatile var writeFd: FileDescriptor? = null
    }

    val isFinished: Boolean get() = finished

    /** Error code that ended the session, so `start` can reject consistently. */
    var terminalError: String? = null
        private set

    // ================= lifecycle =================

    fun start() {
        runGen++
        clearChain()
        segments = 0
        restarts = 0
        segmentedExtra = true
        segmentedSeen = false
        segmentTexts.clear()
        lastPartial = ""
        immediateFails = 0
        finished = false
        userStop = false
        draining = false
        terminalError = null
        synchronized(pipeLock) {
            pcmQueue.clear()
            queueBytes = 0
        }
        synchronized(recorderLock) {
            silenceEnqueued = false
        }
        sink.onState("starting", languageSwitch)
        if (!startRecorder()) {
            finishRun("", "ERROR_AUDIO")
            return
        }
        startFeeder()
        startRecognizer()
    }

    /** User-initiated stop: finish recording, drain the queue, await final. */
    fun stop() {
        if (finished || userStop) return
        userStop = true
        sink.onState("stopping", languageSwitch)
        // The recorder worker may still be inside read(); the silence tail
        // is queued only once the worker can no longer produce PCM, so the
        // tail never overtakes the last speech chunk and the feeder never
        // closes the pipe ahead of buffered audio. If the worker already
        // exited, enqueue it here; otherwise the worker's exit path does.
        var tail = false
        synchronized(recorderLock) {
            // Cleared before AudioRecord.stop() so the worker cannot loop
            // into another read() on a stopped instance and report a
            // spurious fatal error.
            recording = false
            if (recorderExited && !silenceEnqueued) {
                silenceEnqueued = true
                tail = true
            }
        }
        try { audioRecord?.stop() } catch (_: Exception) {}
        if (tail) {
            enqueueSilence()
            draining = true
        }
        scheduleTimeout()
    }

    /** Discard the session (new start replaced it, or user cancelled). */
    fun cancel() {
        teardown()
        sink.onState("idle", languageSwitch)
    }

    /** Activity went to background: teardown plus BACKGROUND_CANCELLED. */
    fun cancelBackground() {
        teardown()
        sink.onError("BACKGROUND_CANCELLED")
        sink.onState("idle", languageSwitch)
    }

    /**
     * Seals the session for a background cancel; safe from any thread.
     * Under [recorderLock] it marks the session finished and stops the
     * AudioRecord if it had started, so a recorder worker that has not
     * passed its start gate yet can never open the microphone — the
     * gate's `recording && !finished` check fails permanently. Returns
     * true when this call did the sealing (the session was still live);
     * the caller must then run [cancelBackground] on the main thread to
     * finish recognizer/event cleanup.
     */
    fun sealForBackground(): Boolean {
        synchronized(recorderLock) {
            if (finished) return false
            finished = true
            recording = false
            try { audioRecord?.stop() } catch (_: Exception) {}
            return true
        }
    }

    private fun teardown() {
        timeout?.let { handler.removeCallbacks(it) }
        timeout = null
        clearChain()
        userStop = true
        synchronized(recorderLock) {
            finished = true
            recording = false
        }
        draining = false
        try { audioRecord?.stop() } catch (_: Exception) {}
        synchronized(pipeLock) {
            pcmQueue.clear()
            queueBytes = 0
        }
        closeActivePipe()
        recognizer?.cancel()
        recognizer?.destroy()
        recognizer = null
    }

    // ================= audio pipeline =================

    private fun startRecorder(): Boolean {
        val minBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        val rec = try {
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION,
                SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minBuf, CHUNK_BYTES * 8),
            )
        } catch (_: Exception) {
            return false
        }
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            try { rec.release() } catch (_: Exception) {}
            return false
        }
        audioRecord = rec
        val gen = runGen
        synchronized(recorderLock) {
            recorderExited = false
            recording = true
        }
        recorderThread = Thread {
            var fatal = false
            var started = false
            // The start gate runs under the same lock that stop(),
            // teardown() and finishRun() write recording/finished through,
            // so a cancel that landed before this thread was scheduled
            // keeps the microphone off.
            synchronized(recorderLock) {
                if (runGen == gen && recording && !finished) {
                    try {
                        rec.startRecording()
                        started = true
                    } catch (_: Exception) {
                        fatal = true
                    }
                }
            }
            val buf = ByteArray(CHUNK_BYTES)
            while (started && !fatal && runGen == gen && recording && !finished) {
                val n = try { rec.read(buf, 0, buf.size) } catch (_: Exception) { -1 }
                if (n > 0) {
                    enqueuePcm(buf, n)
                } else if (n < 0) {
                    fatal = true
                }
            }
            if (started) {
                try { rec.stop() } catch (_: Exception) {}
            }
            try { rec.release() } catch (_: Exception) {}
            audioRecord = null
            // From here on the worker can no longer enqueue PCM. On a user
            // stop the silence tail goes in now, guaranteed after the last
            // recorded chunk, and only then does the feeder start draining.
            var tail = false
            synchronized(recorderLock) {
                if (runGen == gen) {
                    recorderExited = true
                    if (userStop && !finished && !silenceEnqueued) {
                        silenceEnqueued = true
                        tail = true
                    }
                }
            }
            if (tail) {
                enqueueSilence()
                draining = true
            }
            if (fatal) {
                handler.post {
                    if (runGen == gen && !finished) {
                        finishRun(finalText(), "ERROR_AUDIO")
                    }
                }
            }
        }.also { it.isDaemon = true }
        recorderThread?.start()
        return true
    }

    /**
     * Feeder thread: writes queued PCM into the current pipe. The write fd is
     * O_NONBLOCK so a stalled service yields EAGAIN retries instead of a
     * blocked thread; during chain gaps (pipe == null) audio stays queued
     * and is replayed into the next pipe; on drain it closes the write end
     * so the service sees EOF.
     */
    private fun startFeeder() {
        val gen = runGen
        feederThread = Thread {
            var pending: ByteArray? = null
            var pendingOff = 0
            while (runGen == gen && !finished) {
                var chunk = pending
                if (chunk == null) {
                    chunk = synchronized(pipeLock) {
                        pcmQueue.removeFirstOrNull()
                            ?.also { queueBytes -= it.size }
                    }
                    if (chunk != null) {
                        pending = chunk
                        pendingOff = 0
                    }
                }
                val p = synchronized(pipeLock) { pipe }
                if (chunk == null) {
                    if (draining) {
                        if (p != null) {
                            closePipe(p)
                        } else {
                            break
                        }
                    } else {
                        sleepQuiet(15)
                    }
                    continue
                }
                val fd = p?.writeFd
                if (fd == null) {
                    sleepQuiet(15)
                    continue
                }
                try {
                    val n = Os.write(fd, chunk, pendingOff, chunk.size - pendingOff)
                    pendingOff += n
                    if (pendingOff >= chunk.size) {
                        pending = null
                        pendingOff = 0
                    }
                } catch (e: ErrnoException) {
                    if (e.errno == OsConstants.EAGAIN) {
                        sleepQuiet(10)
                    } else {
                        closePipe(p)
                    }
                } catch (_: Exception) {
                    closePipe(p)
                }
            }
        }.also { it.isDaemon = true }
        feederThread?.start()
    }

    private fun enqueuePcm(data: ByteArray, len: Int) {
        val chunk = if (len == data.size) data.copyOf() else data.copyOf(len)
        synchronized(pipeLock) {
            while (queueBytes + chunk.size > QUEUE_MAX_BYTES) {
                val drop = pcmQueue.removeFirstOrNull() ?: break
                queueBytes -= drop.size
            }
            pcmQueue.addLast(chunk)
            queueBytes += chunk.size
        }
    }

    private fun enqueueSilence() {
        val n = (SAMPLE_RATE.toLong() * 2 * STOP_TAIL_MS / 1000).toInt()
        enqueuePcm(ByteArray(n), n)
    }

    private fun createPipe(): SessionPipe? {
        val p = SessionPipe()
        return try {
            val pair = ParcelFileDescriptor.createPipe()
            p.readPfd = pair[0]
            p.writePfd = pair[1]
            val fd = pair[1].fileDescriptor
            p.writeFd = fd
            val fl = Os.fcntlInt(fd, F_GETFL, 0)
            Os.fcntlInt(fd, F_SETFL, fl or O_NONBLOCK)
            p
        } catch (_: Exception) {
            try { p.readPfd?.close() } catch (_: Exception) {}
            try { p.writePfd?.close() } catch (_: Exception) {}
            null
        }
    }

    private fun closePipe(p: SessionPipe) {
        synchronized(pipeLock) {
            if (pipe === p) pipe = null
        }
        p.writeFd = null
        try { p.writePfd?.close() } catch (_: Exception) {}
        try { p.readPfd?.close() } catch (_: Exception) {}
    }

    private fun closeActivePipe() {
        val p = synchronized(pipeLock) { pipe } ?: return
        closePipe(p)
    }

    private fun sleepQuiet(ms: Long) {
        try {
            Thread.sleep(ms)
        } catch (_: InterruptedException) {
        }
    }

    // ================= recognizer =================

    private fun sessionIntent(p: SessionPipe?): Intent {
        val i = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
            .putExtra(
                RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
            )
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
            .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            .putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 3)
            .putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
            .putExtra(RecognizerIntent.EXTRA_CALLING_PACKAGE, context.packageName)
        val readPfd = p?.readPfd
        if (readPfd != null) {
            i.putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE, readPfd)
                .putExtra(RecognizerIntent.EXTRA_AUDIO_SOURCE_CHANNEL_COUNT, 1)
                .putExtra(
                    RecognizerIntent.EXTRA_AUDIO_SOURCE_SAMPLING_RATE,
                    SAMPLE_RATE,
                )
                .putExtra(
                    RecognizerIntent.EXTRA_AUDIO_SOURCE_ENCODING,
                    AudioFormat.ENCODING_PCM_16BIT,
                )
            // EXTRA_SEGMENTED_SESSION holds the *name* of the extra that
            // delimits the session; with EXTRA_AUDIO_SOURCE the session lasts
            // until the app closes the pipe.
            if (segmentedExtra) {
                i.putExtra(
                    RecognizerIntent.EXTRA_SEGMENTED_SESSION,
                    RecognizerIntent.EXTRA_AUDIO_SOURCE,
                )
            }
        }
        if (languageSwitch) {
            i.putExtra(
                RecognizerIntent.EXTRA_ENABLE_LANGUAGE_SWITCH,
                RecognizerIntent.LANGUAGE_SWITCH_BALANCED,
            )
            i.putStringArrayListExtra(
                RecognizerIntent.EXTRA_LANGUAGE_SWITCH_ALLOWED_LANGUAGES,
                arrayListOf("zh-TW", "en-US"),
            )
        }
        return i
    }

    private fun startRecognizer() {
        val rec = try {
            SpeechRecognizer.createOnDeviceSpeechRecognizer(context)
        } catch (_: Exception) {
            finishRun(finalText(), "NOT_AVAILABLE")
            return
        }
        val p = createPipe()
        if (p == null) {
            rec.destroy()
            finishRun(finalText(), "ERROR_AUDIO")
            return
        }
        synchronized(pipeLock) { pipe = p }
        gen++
        recognizer = rec
        lastPartial = ""
        recStartTs = SystemClock.elapsedRealtime()
        rec.setRecognitionListener(listenerFor(rec, gen))
        try {
            rec.startListening(sessionIntent(p))
        } catch (_: Exception) {
            recognizer = null
            rec.destroy()
            closePipe(p)
            finishRun(finalText(), "ERROR_CLIENT")
            return
        }
        // Reset the immediate-fail counter only after the recognizer survives
        // past the failure window; resetting on ready would let a
        // ready->instant-fail loop bypass the circuit breaker.
        val g = gen
        handler.postDelayed({
            if (!finished && gen == g && recognizer === rec) {
                immediateFails = 0
            }
        }, IMMEDIATE_FAIL_MS + 500)
    }

    /**
     * The service ended the session while the user still wants to record:
     * close the old pipe+recognizer and chain into a new pair after a short
     * delay. Recording continues meanwhile; queued audio is replayed into
     * the new pipe so nothing is lost.
     */
    private fun scheduleChain() {
        if (finished || userStop) return
        if (restarts >= MAX_RESTARTS) {
            finishRun(accumulatedText(), "ERROR_TOO_MANY_REQUESTS")
            return
        }
        restarts++
        gen++
        val old = recognizer
        recognizer = null
        closeActivePipe()
        old?.destroy()
        val rg = runGen
        chain?.let { handler.removeCallbacks(it) }
        val c = Runnable {
            chain = null
            if (!finished && runGen == rg) {
                // Even after a user stop the new recognizer is required:
                // it drains the PCM buffered during the chain gap (plus
                // the silence tail) so that speech still reaches the final
                // text instead of being discarded.
                startRecognizer()
                if (userStop && !finished) {
                    // startRecognizer() bumped gen, which would disarm the
                    // timeout scheduled by stop(); rebind it to the new
                    // recognizer so a stalled drain still ends the run.
                    scheduleTimeout()
                }
            }
        }
        chain = c
        handler.postDelayed(c, CHAIN_DELAY_MS)
    }

    private fun clearChain() {
        chain?.let { handler.removeCallbacks(it) }
        chain = null
    }

    private fun scheduleTimeout() {
        timeout?.let { handler.removeCallbacks(it) }
        val g = gen
        val r = Runnable {
            if (!finished && g == gen) {
                finishRun(finalText(), "ERROR_SPEECH_TIMEOUT")
            }
        }
        timeout = r
        handler.postDelayed(r, STOP_TIMEOUT_MS)
    }

    /**
     * Circuit breaker before chaining a service-initiated end: a recognizer
     * that dies inside the immediate window counts as an immediate failure;
     * too many in a row ends the run.
     */
    private fun selfEndAllowed(immediate: Boolean, errorName: String): Boolean {
        if (immediate) {
            immediateFails++
            if (immediateFails >= MAX_IMMEDIATE_FAILS) {
                finishRun(finalText(), errorName)
                return false
            }
        } else {
            immediateFails = 0
        }
        return true
    }

    /** Fold the not-yet-segmented trailing partial into the accumulated text. */
    private fun mergeLastPartial() {
        if (lastPartial.isNotEmpty()) {
            segmentTexts += lastPartial
            lastPartial = ""
        }
    }

    private fun finalText(): String {
        mergeLastPartial()
        return accumulatedText()
    }

    /** Merge service-returned text, tolerating whole-run or repeated tails. */
    private fun mergeFinal(text: String) {
        if (text.isEmpty()) return
        val acc = segmentTexts.joinToString("")
        when {
            text == acc -> {}
            text.startsWith(acc) -> {
                segmentTexts.clear()
                segmentTexts += text
            }
            acc.endsWith(text) -> {}
            else -> segmentTexts += text
        }
    }

    private fun accumulatedText(): String = joinSegmentTexts(segmentTexts)

    private fun finishRun(text: String, error: String?) {
        if (finished) return
        synchronized(recorderLock) {
            finished = true
            recording = false
        }
        draining = false
        timeout?.let { handler.removeCallbacks(it) }
        timeout = null
        clearChain()
        closeActivePipe()
        try { audioRecord?.stop() } catch (_: Exception) {}
        recognizer?.destroy()
        recognizer = null
        if (error != null) {
            terminalError = error
            sink.onError(error)
        } else {
            sink.onFinal(text)
        }
        sink.onState("idle", languageSwitch)
    }

    /** Map SpeechRecognizer codes onto the contract error namespace. */
    private fun mapError(code: Int): String = when (code) {
        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "PERMISSION_DENIED"
        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED,
        SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE,
        -> "LANGUAGE_UNAVAILABLE"
        else -> errorName(code)
    }

    private fun errorName(code: Int): String = when (code) {
        SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "ERROR_NETWORK_TIMEOUT"
        SpeechRecognizer.ERROR_NETWORK -> "ERROR_NETWORK"
        SpeechRecognizer.ERROR_AUDIO -> "ERROR_AUDIO"
        SpeechRecognizer.ERROR_SERVER -> "ERROR_SERVER"
        SpeechRecognizer.ERROR_CLIENT -> "ERROR_CLIENT"
        SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "ERROR_SPEECH_TIMEOUT"
        SpeechRecognizer.ERROR_NO_MATCH -> "ERROR_NO_MATCH"
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "ERROR_RECOGNIZER_BUSY"
        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "ERROR_INSUFFICIENT_PERMISSIONS"
        SpeechRecognizer.ERROR_TOO_MANY_REQUESTS -> "ERROR_TOO_MANY_REQUESTS"
        SpeechRecognizer.ERROR_SERVER_DISCONNECTED -> "ERROR_SERVER_DISCONNECTED"
        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED -> "ERROR_LANGUAGE_NOT_SUPPORTED"
        SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE -> "ERROR_LANGUAGE_UNAVAILABLE"
        SpeechRecognizer.ERROR_CANNOT_CHECK_SUPPORT -> "ERROR_CANNOT_CHECK_SUPPORT"
        SpeechRecognizer.ERROR_CANNOT_LISTEN_TO_DOWNLOAD_EVENTS ->
            "ERROR_CANNOT_LISTEN_TO_DOWNLOAD_EVENTS"
        else -> "ERROR_CODE_$code"
    }

    private fun listenerFor(rec: SpeechRecognizer, g: Int): RecognitionListener =
        object : RecognitionListener {
            /** Drop late callbacks from a superseded recognizer generation. */
            private fun alive() =
                !finished && g == gen && recognizer === rec

            private fun immediateFail(): Boolean =
                SystemClock.elapsedRealtime() - recStartTs < IMMEDIATE_FAIL_MS

            override fun onReadyForSpeech(params: Bundle?) {
                // A recognizer started after a user stop only drains the
                // buffered audio; it must not flip the UI back to
                // "listening".
                if (!alive() || userStop) return
                sink.onState("listening", languageSwitch)
            }

            override fun onBeginningOfSpeech() {}
            override fun onRmsChanged(rmsdB: Float) {}
            override fun onBufferReceived(buffer: ByteArray?) {}

            override fun onEndOfSpeech() {
                // A pause only delimits a segment; it never ends recording.
            }

            override fun onError(error: Int) {
                if (!alive()) return
                val name = mapError(error)
                if (userStop) {
                    finishRun(finalText(), name)
                    return
                }
                when (error) {
                    SpeechRecognizer.ERROR_NO_MATCH,
                    SpeechRecognizer.ERROR_SPEECH_TIMEOUT,
                    SpeechRecognizer.ERROR_SERVER,
                    SpeechRecognizer.ERROR_SERVER_DISCONNECTED,
                    SpeechRecognizer.ERROR_RECOGNIZER_BUSY,
                    -> {
                        // Service ended the session on its own -> chain.
                        if (!selfEndAllowed(immediateFail(), name)) return
                        mergeLastPartial()
                        segments++
                        if (!segmentedSeen) segmentedExtra = false
                        scheduleChain()
                    }
                    else -> {
                        // An immediate failure before any segment was seen may
                        // mean the segmented extra was rejected: retry once
                        // without it before giving up.
                        if (immediateFail() && segmentedExtra &&
                            restarts == 0 && !segmentedSeen
                        ) {
                            immediateFails++
                            segmentedExtra = false
                            mergeLastPartial()
                            scheduleChain()
                        } else {
                            finishRun(finalText(), name)
                        }
                    }
                }
            }

            override fun onResults(results: Bundle?) {
                if (!alive()) return
                val list =
                    results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                var text = list?.firstOrNull { it.isNotEmpty() }
                if (text == null && lastPartial.isNotEmpty()) {
                    // An empty final bundle falls back to the last partial.
                    text = lastPartial
                }
                text = text ?: ""
                if (userStop) {
                    mergeFinal(text)
                    finishRun(accumulatedText(), null)
                } else {
                    // The service ignored the segmented session and ended.
                    if (text.isEmpty() &&
                        !selfEndAllowed(immediateFail(), "ERROR_NO_MATCH")
                    ) {
                        return
                    }
                    if (text.isNotEmpty()) immediateFails = 0
                    segments++
                    if (!segmentedSeen) segmentedExtra = false
                    mergeFinal(text)
                    lastPartial = ""
                    val accumulated = accumulatedText()
                    if (accumulated.isNotEmpty()) {
                        sink.onPartial(accumulated)
                    }
                    scheduleChain()
                }
            }

            override fun onSegmentResults(segmentResults: Bundle) {
                if (!alive()) return
                segmentedSeen = true
                immediateFails = 0
                segments++
                val list = segmentResults.getStringArrayList(
                    SpeechRecognizer.RESULTS_RECOGNITION,
                )
                var text = list?.firstOrNull { it.isNotEmpty() }
                if (text == null && lastPartial.isNotEmpty()) {
                    text = lastPartial
                }
                if (!text.isNullOrEmpty()) {
                    segmentTexts += text
                }
                lastPartial = ""
                val accumulated = accumulatedText()
                if (accumulated.isNotEmpty()) {
                    sink.onPartial(accumulated)
                }
            }

            override fun onEndOfSegmentedSession() {
                if (!alive()) return
                segmentedSeen = true
                if (userStop) {
                    finishRun(finalText(), null)
                } else {
                    if (!selfEndAllowed(immediateFail(), "ERROR_SERVER_DISCONNECTED")) {
                        return
                    }
                    mergeLastPartial()
                    segments++
                    scheduleChain()
                }
            }

            override fun onPartialResults(partialResults: Bundle?) {
                if (!alive()) return
                val list =
                    partialResults?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                if (!list.isNullOrEmpty() && list[0].isNotEmpty()) {
                    lastPartial = list[0]
                    val text = joinSegmentTexts(segmentTexts + lastPartial)
                    if (text.isNotEmpty()) {
                        sink.onPartial(text)
                    }
                }
            }

            override fun onEvent(eventType: Int, params: Bundle?) {}
        }
}
