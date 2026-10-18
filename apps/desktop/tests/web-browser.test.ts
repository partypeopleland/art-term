import { afterEach, expect, it, vi } from 'vitest'
import { createNativeWebLinkOpener } from '../src/transport/browser'
import { openPreviewWebUrl } from '../src/terminal/links'
const { invoke }=vi.hoisted(()=>({invoke:vi.fn(async()=>{})}))
vi.mock('@tauri-apps/api/core',()=>({invoke}))
afterEach(()=>{vi.restoreAllMocks();invoke.mockClear()})
it.each([['windows','gui_open_web_url'],['android','plugin:mobile-history|openUrl']] as const)('uses the %s platform browser command with the unchanged URL',async(platform,command)=>{
 const url='https://example.com/中文?q=a%20b#fragment'
 await createNativeWebLinkOpener(platform)(url)
 expect(invoke).toHaveBeenCalledExactlyOnceWith(command,{url})
})
it('opens preview synchronously with isolated browsing and accepts noopener null',()=>{
 const open=vi.spyOn(window,'open').mockReturnValue(null)
 expect(()=>openPreviewWebUrl('http://example.com/')).not.toThrow()
 expect(open).toHaveBeenCalledExactlyOnceWith('http://example.com/','_blank','noopener,noreferrer')
})
