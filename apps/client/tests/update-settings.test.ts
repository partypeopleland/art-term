import { describe, it, expect, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import UpdateSettings from '../src/shared/components/UpdateSettings.vue'
import type { Transport, UpdateCheckDto, UpdateInstallDto } from '../src/shared/transport/types'

function transportWith(overrides: Partial<Transport>): Transport {
  return {
    open: vi.fn(),
    poll: vi.fn(),
    write: vi.fn(),
    resize: vi.fn(),
    close: vi.fn(),
    desktop: vi.fn(),
    screenshot: vi.fn(),
    ...overrides
  } as Transport
}

const checkResult: UpdateCheckDto = {
  current: '0.1.38',
  latest: '0.1.39',
  available: true,
  message: 'release notes'
}

describe('UpdateSettings', () => {
  it('does not check for updates until the button is pressed', async () => {
    const updateCheck = vi.fn().mockResolvedValue(checkResult)
    const wrapper = mount(UpdateSettings, {
      props: { transport: transportWith({ updateCheck, updateInstall: vi.fn() }) }
    })
    await nextTick()
    expect(updateCheck).not.toHaveBeenCalled()
    await wrapper.get('[data-testid="update-check"]').trigger('click')
    await vi.waitFor(() => expect(updateCheck).toHaveBeenCalledTimes(1))
    expect(wrapper.text()).toContain('0.1.38')
    expect(wrapper.text()).toContain('0.1.39')
    expect(wrapper.get('[data-testid="update-install"]').text()).toContain('0.1.39')
  })

  it('passes the confirmed latest version to install', async () => {
    const updateCheck = vi.fn().mockResolvedValue(checkResult)
    const updateInstall = vi
      .fn()
      .mockResolvedValue({ status: 'installing', version: '0.1.39' } as UpdateInstallDto)
    const wrapper = mount(UpdateSettings, {
      props: { transport: transportWith({ updateCheck, updateInstall }) }
    })
    await wrapper.get('[data-testid="update-check"]').trigger('click')
    await vi.waitFor(() => expect(wrapper.find('[data-testid="update-install"]').exists()).toBe(true))
    await wrapper.get('[data-testid="update-install"]').trigger('click')
    await vi.waitFor(() => expect(updateInstall).toHaveBeenCalledWith('0.1.39'))
    expect(wrapper.text()).toContain('系統安裝程式')
  })

  it('surfaces check errors without offering install', async () => {
    const updateCheck = vi.fn().mockRejectedValue(new Error('network down'))
    const wrapper = mount(UpdateSettings, {
      props: { transport: transportWith({ updateCheck }) }
    })
    await wrapper.get('[data-testid="update-check"]').trigger('click')
    await vi.waitFor(() => expect(wrapper.text()).toContain('network down'))
    expect(wrapper.find('[data-testid="update-install"]').exists()).toBe(false)
  })
})
