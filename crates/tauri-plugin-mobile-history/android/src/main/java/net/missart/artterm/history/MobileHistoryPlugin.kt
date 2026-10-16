package net.missart.artterm.history

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import android.graphics.Color
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.text.style.BackgroundColorSpan
import android.text.style.StyleSpan
import android.text.style.UnderlineSpan
import android.text.style.StrikethroughSpan
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.ViewTreeObserver
import android.webkit.WebView
import android.widget.FrameLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class RecordArgs {
    var id: Long = 0
    var wrapped: Boolean = false
    var text: String = ""
    var styles: Array<IntArray> = emptyArray()
}
@InvokeArg
class BoundsArgs {
    var x: Double = 0.0
    var y: Double = 0.0
    var width: Double = 0.0
    var height: Double = 0.0
    var viewportWidth: Double = 0.0
}
@InvokeArg
class ReaderArgs {
    var readerId: String = ""
    var records: Array<RecordArgs> = emptyArray()
    var visible: Boolean = true
    var followLatest: Boolean = true
    var sourceEpoch: String? = null
    var bounds: BoundsArgs? = null
}

/** Local terminal projection: ScrollView + one TextView.
 * Embedded over the conversation rectangle; input/toolbars remain in the WebView.
 * Android alone owns drag/fling. No wheel synthesis or per-frame JS messages. */
@TauriPlugin
class MobileHistoryPlugin(private val activity: Activity) : Plugin(activity) {
    private val main = Handler(Looper.getMainLooper())
    private var webView: WebView? = null
    private var root: FrameLayout? = null
    private var readerId = ""
    private var scroll: ScrollView? = null
    private var text: TextView? = null
    private var starts = linkedMapOf<Long, Int>()
    private var renderedStyles: List<List<Int>> = emptyList()
    private var adjusting = false
    private var dragging = false
    private var moving = false
    private var userGesture = false
    private var userViewportPending = false
    private var pageVisible = false
    private var epoch = 0
    private var pending: ReaderArgs? = null
    private var layoutPending = false
    private var layoutAnchor: Anchor? = null
    private var layoutBottom = false
    private var pendingDraw: Pair<View, ViewTreeObserver.OnPreDrawListener>? = null
    private data class Link(val start: Int, val end: Int, val url: String)
    private data class Press(val link: Link?, val x: Float, val y: Float, val time: Long)
    private var links = emptyList<Link>()
    private var press: Press? = null
    private var cssScale = activity.resources.displayMetrics.density.toDouble()
    private fun findLinks(value: String): List<Link> {
        return Regex("""https?://[^\s<>"'`]+""", RegexOption.IGNORE_CASE).findAll(value).mapNotNull { match ->
            var url = match.value.trimEnd('.', ',', ';', ':', '!', '?', '，', '。', '；', '！', '？', '、', '”', '》')
            while (url.lastOrNull() in listOf(')', ']', '}')) {
                val close = url.last(); val open = when (close) { ')' -> '('; ']' -> '['; else -> '{' }
                if (url.count { it == close } <= url.count { it == open }) break
                url = url.dropLast(1)
            }
            if (android.net.Uri.parse(url).host.isNullOrEmpty()) null
            else Link(match.range.first, match.range.first + url.length, url)
        }.toList()
    }
    private fun linkAt(x: Float, y: Float): Link? {
        val t = text ?: return null; val s = scroll ?: return null; val layout = t.layout ?: return null
        val tx = x - t.left - t.totalPaddingLeft
        val ty = y + s.scrollY - t.top - t.totalPaddingTop
        if (ty < 0 || ty >= layout.height) return null
        val line = layout.getLineForVertical(ty.toInt())
        if (tx < layout.getLineLeft(line) || tx >= layout.getLineRight(line)) return null
        val character = layout.getOffsetForHorizontal(line, tx)
        return links.firstOrNull { character >= it.start && character < it.end }
    }
    private fun touch(event: MotionEvent) {
        val slop = android.view.ViewConfiguration.get(activity).scaledTouchSlop
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> press = if (!moving) Press(linkAt(event.x, event.y), event.x, event.y, event.eventTime) else null
            MotionEvent.ACTION_MOVE -> press?.let {
                if (kotlin.math.hypot((event.x - it.x).toDouble(), (event.y - it.y).toDouble()) >= slop) press = null
            }
            MotionEvent.ACTION_UP -> {
                val p = press; press = null
                if (p != null && event.eventTime - p.time < android.view.ViewConfiguration.getLongPressTimeout()
                    && kotlin.math.hypot((event.x - p.x).toDouble(), (event.y - p.y).toDouble()) < slop) {
                    if (p.link == null) send("tap")
                    else if (linkAt(event.x, event.y) == p.link) {
                        try {
                            (activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
                                .setPrimaryClip(ClipData.newPlainText("URL", p.link.url))
                            Toast.makeText(activity, "已複製連結", Toast.LENGTH_SHORT).show()
                        } catch (_: Exception) { Toast.makeText(activity, "複製失敗，請再點一次連結", Toast.LENGTH_SHORT).show() }
                    }
                }
            }
            MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN -> press = null
        }
    }
    private data class Anchor(val id: Long, val character: Int, val pixel: Int)
    private var sourceEpoch: String? = null
    private var pendingAnchor: Anchor? = null
    private var pendingFollow = false
    private var pendingSourceEpoch: String? = null
    private var committedPage: ReaderArgs? = null
    private var gestureStartY = 0
    private var gestureTouchY = 0f
    private var upwardIntent = false
    private val settled = Runnable {
        if (!dragging) {
            moving = false
            userGesture = false
            if (userViewportPending) {
                userViewportPending = false
                anchor()?.let { a -> send("viewport") {
                    put("anchorId", a.id)
                    put("anchorCharacter", a.character)
                    put("anchorPixel", a.pixel)
                    put("firstId", starts.keys.firstOrNull())
                    put("lastId", starts.keys.lastOrNull())
                    put("atBottom", atBottom())
                    put("nearTop", (scroll?.scrollY ?: 0) <= dp(64) && (upwardIntent || (scroll?.scrollY ?: 0) < gestureStartY))
                    put("nearBottom", (text?.height ?: 0) - (scroll?.height ?: 0) - (scroll?.scrollY ?: 0) <= dp(64))
                } }
            }
            val page = pending
            pending = null
            if (page != null) applyPage(page)
        }
    }
    private var lastKeyboardVisible: Boolean? = null
    private fun keyboardVisible(): Boolean = webView?.let {
        ViewCompat.getRootWindowInsets(it)?.isVisible(WindowInsetsCompat.Type.ime())
    } ?: false
    private val keyboardLayout = ViewTreeObserver.OnGlobalLayoutListener {
        val visible = keyboardVisible()
        if (visible != lastKeyboardVisible) {
            lastKeyboardVisible = visible
            trigger("keyboard", JSObject().apply { put("visible", visible) })
        }
    }
    override fun load(webView: WebView) {
        this.webView = webView
        webView.viewTreeObserver.addOnGlobalLayoutListener(keyboardLayout)
    }
    @Command
    fun keyboardStatus(invoke: Invoke) {
        main.post { invoke.resolve(JSObject().apply { put("visible", keyboardVisible()) }) }
    }
    private fun dp(value: Int) = (value * activity.resources.displayMetrics.density).toInt()
    private fun send(kind: String, id: String = readerId, fill: JSObject.() -> Unit = {}) {
        if (id.isEmpty()) return
        trigger("history", JSObject().apply { put("readerId", id); put("kind", kind); sourceEpoch?.let { put("sourceEpoch", it) }; fill() })
    }
    private fun anchor(): Anchor? {
        val s = scroll ?: return null
        val t = text ?: return null
        val layout = t.layout ?: return null
        if (starts.isEmpty()) return null
        val line = layout.getLineForVertical((s.scrollY - t.top - t.totalPaddingTop).coerceAtLeast(0))
        val character = layout.getLineStart(line)
        val entry = starts.entries.lastOrNull { it.value <= character } ?: starts.entries.first()
        return Anchor(entry.key, (character - entry.value).coerceAtLeast(0),
            t.top + t.totalPaddingTop + layout.getLineTop(line) - s.scrollY)
    }
    private fun atBottom(): Boolean {
        val s = scroll ?: return true
        return (text?.height ?: 0) - s.height - s.scrollY <= dp(2)
    }
    private fun cancelDraw() {
        pendingDraw?.let { (view, listener) ->
            if (view.viewTreeObserver.isAlive) view.viewTreeObserver.removeOnPreDrawListener(listener)
        }
        pendingDraw = null; adjusting = false
    }
    private fun closeReader() {
        epoch++; cancelDraw(); main.removeCallbacks(settled)
        root?.let { (it.parent as? ViewGroup)?.removeView(it) }
        root = null; scroll = null; text = null
        starts.clear(); renderedStyles = emptyList(); readerId = ""; pending = null
        links = emptyList(); press = null
        dragging = false; moving = false; userGesture = false; userViewportPending = false; pageVisible = false
        sourceEpoch = null; pendingAnchor = null; pendingFollow = false
        pendingSourceEpoch = null; committedPage = null
        layoutPending = false; layoutAnchor = null; layoutBottom = false
    }
    private fun validStyles(value: String, styles: Array<IntArray>, cap: Int): Boolean {
        if (styles.size > cap) return false
        var previous = 0
        fun boundary(i: Int) = i in 0..value.length && (i == value.length || !Character.isLowSurrogate(value[i]))
        for (run in styles) {
            if (run.size != 5) return false
            val (start, end, fg, bg, flags) = run
            if (start < previous || start >= end || !boundary(start) || !boundary(end)
                || fg !in -1..0xffffff || bg !in -1..0xffffff || flags !in 0..127) return false
            previous = end
        }
        return true
    }
    private fun valid(args: ReaderArgs): Boolean {
        if (!args.readerId.matches(Regex("[a-zA-Z0-9-]{1,80}")) || args.records.size > 8192) return false
        if (args.sourceEpoch != null && !args.sourceEpoch!!.matches(Regex("[a-f0-9]{32}:[1-9][0-9]{0,15}"))) return false
        var last = 0L
        var size = 0L
        for (r in args.records) {
            if (r.id <= last || r.id > 9007199254740991L || r.text.length > 8192
                || !validStyles(r.text, r.styles, 8192)) return false
            last = r.id; size += r.text.length * 2L + 32L + r.styles.size * 40L
        }
        val b = args.bounds
        if (b != null && listOf(b.x, b.y, b.width, b.height, b.viewportWidth).any { !it.isFinite() || it < 0 || it > 100000 }) return false
        return size <= 1280L * 1024L
    }
    private fun place(args: ReaderArgs) {
        // Save the actual reading state before Android receives new bounds.
        // Text may be identical during IME resize, but scroll limits change.
        if (!layoutPending) {
            layoutAnchor = if (adjusting) pendingAnchor else anchor()
            layoutBottom = if (adjusting) pendingFollow else atBottom()
        }
        val r = root ?: return
        val web = webView ?: return
        val parent = r.parent as? ViewGroup ?: return
        val b = args.bounds
        pageVisible = args.visible && (b == null || (b.width > 0 && b.height > 0 && b.viewportWidth > 0))
        if (!args.visible || (b != null && (b.width <= 0 || b.height <= 0 || b.viewportWidth <= 0))) {
            layoutPending = true
            r.visibility = View.GONE
            press = null
            return
        }
        val webPos = IntArray(2); web.getLocationOnScreen(webPos)
        val parentPos = IntArray(2); parent.getLocationOnScreen(parentPos)
        val scale = if (b != null) web.width / b.viewportWidth else 1.0
        cssScale = scale
        // FrameLayout places margins inside its system-inset padding. The
        // WebView screen origin already includes those insets; subtract them
        // here instead of applying the status/navigation-bar padding twice.
        val x = webPos[0] - parentPos[0] - parent.paddingLeft + ((b?.x ?: 0.0) * scale).toInt()
        val y = webPos[1] - parentPos[1] - parent.paddingTop + ((b?.y ?: 0.0) * scale).toInt()
        val w = ((b?.width ?: web.width.toDouble()) * scale).toInt().coerceIn(0, web.width)
        val h = ((b?.height ?: web.height.toDouble()) * scale).toInt().coerceIn(0, web.height)
        val params = r.layoutParams as FrameLayout.LayoutParams
        if (params.width != w || params.height != h || params.leftMargin != x || params.topMargin != y) {
            layoutPending = true
            r.layoutParams = FrameLayout.LayoutParams(w, h).apply { leftMargin = x; topMargin = y }
        }
        if (r.visibility != View.VISIBLE) layoutPending = true
        r.visibility = View.VISIBLE
    }
    @Command
    fun open(invoke: Invoke) {
        val args = invoke.parseArgs(ReaderArgs::class.java)
        if (!valid(args)) { invoke.reject("Invalid local terminal projection"); return }
        main.post {
            val parent = activity.findViewById<ViewGroup>(android.R.id.content)
            if (activity.isFinishing || activity.isDestroyed || webView == null || parent !is FrameLayout) {
                invoke.reject("Native terminal view unavailable"); return@post
            }
            closeReader(); readerId = args.readerId
            val frame = FrameLayout(activity).apply { setBackgroundColor(Color.rgb(22,23,27)) }
            val s = object : ScrollView(activity) {
                override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                    touch(event)
                    when (event.actionMasked) {
                        MotionEvent.ACTION_DOWN -> {
                            dragging = true; moving = true; userGesture = true
                            gestureStartY = scrollY; gestureTouchY = event.y; upwardIntent = false
                            userViewportPending = true
                            // A candidate can arrive just before the input
                            // frame. Restore the actual committed epoch before
                            // its pending draw can replace the touched page.
                            if (adjusting && pendingSourceEpoch != sourceEpoch) committedPage?.let { applyPage(it) }
                            send("reading")
                            main.removeCallbacks(settled)
                        }
                        MotionEvent.ACTION_MOVE -> {
                            if (event.y - gestureTouchY > android.view.ViewConfiguration.get(activity).scaledTouchSlop) upwardIntent = true
                        }
                        MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                            dragging = false
                            main.removeCallbacks(settled); main.postDelayed(settled, 180)
                        }
                    }
                    return super.dispatchTouchEvent(event)
                }
            }.apply {
                isFillViewport = true
                isFocusable = false; isFocusableInTouchMode = false
                descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS
            }
            val t = TextView(activity).apply {
                textSize = 14f; typeface = Typeface.MONOSPACE
                setTextColor(Color.rgb(215,215,220)); setLineSpacing(0f, 1.2f)
                setPadding(dp(12), dp(8), dp(12), dp(24))
                isFocusable = false; isFocusableInTouchMode = false
                setTextIsSelectable(false)
                isLongClickable = false
            }
            s.addView(t, ViewGroup.LayoutParams(-1, -2))
            frame.addView(s, FrameLayout.LayoutParams(-1, -1))
            parent.addView(frame, FrameLayout.LayoutParams(0, 0))
            root = frame; scroll = s; text = t
            s.setOnScrollChangeListener { _, _, _, _, _ ->
                if (!adjusting && userGesture) {
                    userViewportPending = true
                    moving = true
                    main.removeCallbacks(settled); main.postDelayed(settled, 180)
                }
            }
            place(args); applyPage(args)
            invoke.resolve()
        }
    }
    @Command
    fun update(invoke: Invoke) {
        val args = invoke.parseArgs(ReaderArgs::class.java)
        if (!valid(args)) { invoke.reject("Invalid local terminal projection"); return }
        main.post {
            if (args.readerId == readerId) {
                place(args)
                if (dragging || moving) pending = args else applyPage(args)
            }
            invoke.resolve()
        }
    }
    private fun applyPage(args: ReaderArgs) {
        if (!pageVisible) return
        val s = scroll ?: return
        val t = text ?: return
        val nextStarts = linkedMapOf<Long, Int>()
        val content = StringBuilder()
        val nextStyles = mutableListOf<List<Int>>()
        fun append(value: String, styles: Array<IntArray>) {
            val offset = content.length
            content.append(value)
            for (run in styles) nextStyles.add(listOf(offset + run[0], offset + run[1], run[2], run[3], run[4]))
        }
        for ((index, r) in args.records.withIndex()) {
            if (index > 0 && !r.wrapped) content.append('\n')
            nextStarts[r.id] = content.length
            append(r.text, r.styles)
        }
        val nextText = content.toString()
        if (!layoutPending && !adjusting && t.text.toString() == nextText && renderedStyles == nextStyles && starts == nextStarts && sourceEpoch == args.sourceEpoch) { starts = nextStarts; return }
        press = null; links = findLinks(nextText)
        val a = if (adjusting) pendingAnchor else if (layoutPending) layoutAnchor else anchor()
        val follow = args.followLatest && (if (adjusting) pendingFollow else if (layoutPending) layoutBottom else atBottom()) && !dragging && !moving
        cancelDraw(); val generation = ++epoch; adjusting = true
        pendingAnchor = a; pendingFollow = follow
        pendingSourceEpoch = args.sourceEpoch
        starts = nextStarts; renderedStyles = nextStyles
        val styled = SpannableStringBuilder(nextText)
        for ((start, end, rawFg, rawBg, flags) in nextStyles) {
            var fg = if (rawFg < 0) 0xd7d7dc else rawFg
            var bg = if (rawBg < 0) 0x16171b else rawBg
            if (flags and 32 != 0) { val swap = fg; fg = bg; bg = swap }
            if (flags and 16 != 0) {
                fg = Color.rgb(((fg shr 16 and 255) + (bg shr 16 and 255)) / 2,
                    ((fg shr 8 and 255) + (bg shr 8 and 255)) / 2, ((fg and 255) + (bg and 255)) / 2) and 0xffffff
            }
            if (flags and 64 != 0) fg = bg
            fun span(value: Any) = styled.setSpan(value, start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            span(ForegroundColorSpan(fg or (0xff shl 24)))
            span(BackgroundColorSpan(bg or (0xff shl 24)))
            val style = (if (flags and 1 != 0) Typeface.BOLD else 0) or (if (flags and 2 != 0) Typeface.ITALIC else 0)
            if (style != 0) span(StyleSpan(style))
            if (flags and 4 != 0) span(UnderlineSpan())
            if (flags and 8 != 0) span(StrikethroughSpan())
        }
        t.text = styled
        val draw = object : ViewTreeObserver.OnPreDrawListener {
            override fun onPreDraw(): Boolean {
                if (t.viewTreeObserver.isAlive) t.viewTreeObserver.removeOnPreDrawListener(this)
                if (generation == epoch && readerId == args.readerId) {
                    pendingDraw = null
                    if (follow) s.scrollTo(0, (t.height - s.height).coerceAtLeast(0))
                    else if (a != null && !starts.containsKey(a.id)) s.scrollTo(0, 0)
                    else if (a != null) starts[a.id]?.let { start -> t.layout?.let { layout ->
                        val line = layout.getLineForOffset((start + a.character).coerceAtMost(t.length()))
                        s.scrollTo(0, (t.top + t.totalPaddingTop + layout.getLineTop(line) - a.pixel).coerceAtLeast(0))
                    }
                    }
                    adjusting = false
                    layoutPending = false; layoutAnchor = null; layoutBottom = false
                    pendingAnchor = null; pendingFollow = false; pendingSourceEpoch = null
                    sourceEpoch = args.sourceEpoch; committedPage = args
                    // A read-only layout result, distinct from user viewport
                    // requests. It must never request another history page.
                    anchor()?.let { current -> send("position") {
                        put("anchorId", current.id)
                        put("anchorCharacter", current.character)
                        put("anchorPixel", current.pixel)
                        put("firstId", starts.keys.firstOrNull())
                        put("lastId", starts.keys.lastOrNull())
                        // Prepending a soft-wrapped prefix can change the
                        // first visible line. Observe the saved character's
                        // actual layout position rather than comparing those
                        // two different first-visible line identities.
                        if (a != null) starts[a.id]?.let { start -> t.layout?.let { layout ->
                            val line = layout.getLineForOffset((start + a.character).coerceAtMost(t.length()))
                            put("restoredAnchorId", a.id)
                            put("restoredAnchorCharacter", a.character)
                            put("restoredAnchorPixel", t.top + t.totalPaddingTop + layout.getLineTop(line) - s.scrollY)
                        } }
                    } }
                    main.removeCallbacks(settled); main.postDelayed(settled, 180)
                }
                return true
            }
        }
        pendingDraw = Pair(t, draw); t.viewTreeObserver.addOnPreDrawListener(draw); t.requestLayout()
    }
    @Command
    fun close(invoke: Invoke) {
        val args = invoke.parseArgs(ReaderArgs::class.java)
        main.post { if (args.readerId == readerId) closeReader(); invoke.resolve() }
    }
    override fun onPause(activity: AppCompatActivity) {
        press = null
        root?.visibility = View.GONE
        send("visibility") { put("visible", false) }
    }
    override fun onResume(activity: AppCompatActivity) {
        root?.visibility = if (pageVisible) View.VISIBLE else View.GONE
        send("visibility") { put("visible", true) }
    }
    override fun onDestroy(activity: AppCompatActivity) {
        webView?.viewTreeObserver?.takeIf { it.isAlive }?.removeOnGlobalLayoutListener(keyboardLayout)
        closeReader()
    }
}
