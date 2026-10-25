import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import AppHomepage from '../src/shared/components/AppHomepage.vue'
import appInfo from 'virtual:art-term-info'

it.each([
  ['linux', 'Linux', appInfo.windowsVersion],
  ['windows', 'Windows', appInfo.windowsVersion],
  ['android', `Android · Build ${appInfo.androidBuild}`, appInfo.version]
] as const)('shows the %s platform and its packaged version', (platform, label, version) => {
  const wrapper = mount(AppHomepage, { props: { platform } })
  expect(wrapper.get('.version-value').text()).toBe(`${version}${label}`)
  wrapper.unmount()
})
