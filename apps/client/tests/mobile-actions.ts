import { nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { vi } from 'vitest'

export async function flushMobile(): Promise<void> {
  for (let i = 0; i < 50; i++) { await Promise.resolve(); await nextTick() }
}

/** The fixed Main/Connect navigation commits after its slide transition. */
export async function showMobileMain(wrapper: ReturnType<typeof mount>): Promise<void> {
  await wrapper.get('[data-testid="terminal-to-list"]').trigger('click')
  if (vi.isFakeTimers()) await vi.advanceTimersByTimeAsync(220)
  else await new Promise(resolve => setTimeout(resolve, 220))
  await flushMobile()
}

/** Attach the explicitly displayed Shell; the Host label only refreshes/expands. */
export async function attachMobileProfile(wrapper: ReturnType<typeof mount>, label: string): Promise<void> {
  const row = wrapper.findAll('.host-connections').find(row => row.find(`[data-runtime-label="${label}"]`).exists())
  if (!row) throw new Error(`Missing mobile Host: ${label}`)
  const summary = row.find('.connection-summary')
  if (summary.exists()) await summary.trigger('click')
  await row.get('.attach-button').trigger('click')
  await flushMobile()
}
