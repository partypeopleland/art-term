// @vitest-environment node
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const kotlin = readFileSync(resolve(process.cwd(), '../../crates/tauri-plugin-mobile-history/android/src/main/java/net/missart/artterm/history/MobileHistoryPlugin.kt'), 'utf8')
function section(start: string, end: string): string {
  const from = kotlin.indexOf(start)
  const to = kotlin.indexOf(end, from + start.length)
  expect(from).toBeGreaterThanOrEqual(0)
  expect(to).toBeGreaterThan(from)
  return kotlin.slice(from, to)
}

describe('Android history swipe lifecycle', () => {
  it('cancels the settle animator before resetting, removing, or restarting the reader view', () => {
    const settle = section('private fun settleSwipe', 'private fun cancelDraw()')
    expect(settle.indexOf('animate()?.cancel()')).toBeGreaterThanOrEqual(0)
    expect(settle.indexOf('animate()?.cancel()')).toBeLessThan(settle.indexOf('translationX(target)'))

    const pause = section('override fun onPause', 'override fun onResume')
    expect(pause.indexOf('animate()?.cancel()')).toBeGreaterThanOrEqual(0)
    expect(pause.indexOf('animate()?.cancel()')).toBeLessThan(pause.indexOf('translationX = 0f'))

    const close = section('private fun closeReader()', 'private fun validStyles')
    expect(close.indexOf('animate()?.cancel()')).toBeGreaterThanOrEqual(0)
    expect(close.indexOf('animate()?.cancel()')).toBeLessThan(close.indexOf('removeView'))

    const restart = section('horizontalSwipe = true; dragging = false', 'if (horizontalSwipe) {')
    expect(restart.indexOf('animate()?.cancel()')).toBeGreaterThanOrEqual(0)
    expect(restart.indexOf('animate()?.cancel()')).toBeLessThan(restart.indexOf('send("swipe")'))
  })
})
