import { mount, type VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, expect, it, vi } from 'vitest'
import type { ITheme } from '@xterm/xterm'
import App from '../src/App.vue'
import type { AppController } from '../src/state/controller'
import type { HostHistoryReply, HostHistoryRequest } from '../src/terminal/native'
import { selectedTheme, selectTheme, terminalTheme } from '../src/mobile/theme'
import { FakeTerm, FakeTransport, makeHost, makeProfile, makeTarget, deferred } from './fake'
vi.stubGlobal('ResizeObserver',class { observe() {} disconnect() {} })
const wrappers:VueWrapper[]=[]
afterEach(()=>{for(const wrapper of wrappers.splice(0))wrapper.unmount();document.body.replaceChildren();selectTheme('tech');vi.useRealTimers()})
async function flush(){for(let i=0;i<80;i++){await Promise.resolve();await nextTick()}}
class ThemeTerm extends FakeTerm { colors:ITheme[]=[];setTheme(theme:ITheme){this.colors.push(theme)} }
const screen={id:'screen-A',revision:1,name:'Screen A',mcpUrl:'http://127.0.0.1:9000/mcp',tokenPresent:true}
class WindowsTransport extends FakeTransport {
 instanceDesktops=vi.fn(async()=>[screen,{...screen,id:'screen-B',name:'Screen B'}])
 instanceSelect=vi.fn(async(sid:string,id:string)=>{this.shot={...this.shot,desktopId:id,desktopRevision:1};return {desktop:{...screen,id,name:id==='screen-A'?'Screen A':'Screen B'},toolReady:false}})
 instanceClear=vi.fn(async()=>{})
 runtimeCreateHistorySession=vi.fn(async(id:string)=>makeTarget('new-history-shell',id))
 runtimeHistory=vi.fn(async(_id:string,_request:HostHistoryRequest):Promise<HostHistoryReply>=>({kind:'unsupported'}))
}
async function setup(){
 const target=makeTarget('shell','host','host',true,false)
 const transport=new WindowsTransport([makeProfile('host','My computer')]);transport.runtimeSessionsData.set('host',makeHost('host',[target]))
 const terms:ThemeTerm[]=[]
 const wrapper=mount(App,{props:{transport,terminalFactory:()=>{const term=new ThemeTerm();terms.push(term);return term},controllerOptions:{idlePollMs:0}},attachTo:document.body});wrappers.push(wrapper)
 await flush();const controller=(wrapper.vm as unknown as {controller:AppController}).controller
 async function attach(){await wrapper.get('[data-testid="runtime-host"] .attach-button').trigger('click');await flush();return controller.tabs[0]}
 return {wrapper,controller,transport,terms,target,attach}
}
it('lists inline Shells without opening and creates a new history Shell only on plus',async()=>{
 const {wrapper,controller,transport}=await setup();expect(transport.openCalls).toEqual([])
 expect(wrapper.find('[data-testid="cat-screens"]').exists()).toBe(true)
 expect(wrapper.find('[data-testid="screen-screen-A"]').exists()).toBe(true)
 await wrapper.get('[data-testid="runtime-host"] .new-connection').trigger('click');await flush()
 expect(transport.runtimeCreateHistorySession).toHaveBeenCalledWith('host',1)
 expect(transport.openCalls[0].targetId).toBe('new-history-shell');expect(controller.tabs[0].status).toBe('live')
})
it('keeps terminal mounted and draining under full settings and homepage; restores the combined workspace',async()=>{
 const {wrapper,controller,transport,terms,attach}=await setup();const tab=await attach();const sid=tab.sessionId!;const term=terms[0]
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush();const focus=term.focusCount
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush()
 expect(wrapper.find('.desktop-settings-page').exists()).toBe(true);expect(terms).toHaveLength(1);expect(term.disposed).toBe(false)
 transport.feed(sid,{bytes:[72,105],exitCode:null});await flush();expect(term.text()).toBe('Hi');expect(term.focusCount).toBe(focus)
 expect(wrapper.find('[data-testid="desktop-manage-hosts"]').exists()).toBe(false)
 expect(wrapper.find('[data-testid="screen-settings"]').exists()).toBe(false)
 await wrapper.get('[data-testid="settings-screens"]').trigger('click');await flush()
 expect(wrapper.find('[data-testid="screen-settings"]').exists()).toBe(true)
 await wrapper.get('[aria-label="軟體首頁"]').trigger('click');await flush();expect(wrapper.get('.version-value').text()).toContain('Windows');expect(wrapper.get('.version-value').text()).not.toContain('Android')
 await wrapper.get('[data-testid="desktop-workspace"]').trigger('click');await flush();expect(transport.closeCalls).toEqual([])
 expect(term.focusCount).toBeGreaterThan(focus);expect(term.fitCount).toBeGreaterThan(1);expect(controller.tabs[0]).toBe(tab)
 expect(wrapper.get('.desktop-sidebar .desktop-main-page').isVisible()).toBe(true)
 expect(wrapper.get('.desktop-connect-page').isVisible()).toBe(true)
 expect(wrapper.find('[data-testid="desktop-main"]').exists()).toBe(false)
 expect(wrapper.find('[data-testid="desktop-connect"]').exists()).toBe(false)
})
it('applies all four themes immediately to existing and future terminals',async()=>{
 const {wrapper,controller,terms,attach}=await setup();await attach()
 for(const theme of ['dark','light','retro','tech'] as const){selectTheme(theme);await flush();expect(wrapper.attributes('data-theme')).toBe(theme);expect(terms[0].colors.at(-1)).toEqual(terminalTheme(theme))}
 selectTheme('retro');await flush();const next=makeTarget('other','host');controller.openTarget({dto:makeHost('host',[next]),expanded:true},next,false);await flush();expect(terms[1].colors[0]).toEqual(terminalTheme('retro'))
})
it('selects/clears independent Screens on a host without legacy capability and isolates two tabs',async()=>{
 const {wrapper,controller,transport,attach}=await setup();const first=await attach()
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click');await flush();await wrapper.get('[data-testid="choose-screen-screen-A"]').trigger('click');await flush()
 expect(first.instanceDesktop?.id).toBe('screen-A');expect(first.previewVisible).toBe(true);expect(first.shot).not.toBeNull();expect(transport.instanceSelect).toHaveBeenCalledWith(first.sessionId,'screen-A',1)
 const other=makeTarget('other','host');controller.openTarget({dto:makeHost('host',[other]),expanded:true},other,false);await flush();const second=controller.activeTab.value!
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click');await flush();await wrapper.get('[data-testid="choose-screen-screen-B"]').trigger('click');await flush();expect(second.instanceDesktop?.id).toBe('screen-B');expect(first.instanceDesktop?.id).toBe('screen-A')
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click');await flush();await wrapper.get('[data-testid="clear-screen"]').trigger('click');await flush();expect(second.instanceDesktop).toBeNull();expect(first.instanceDesktop?.id).toBe('screen-A')
})
it('stops previews under settings, restores prior visibility and never clears tools',async()=>{
 const {wrapper,controller,transport,attach}=await setup();const tab=await attach();await controller.selectInstance(tab,screen);await flush();expect(tab.previewVisible).toBe(true)
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush();expect(tab.previewVisible).toBe(false);expect(tab.instanceDesktop?.id).toBe(screen.id)
 const captured=transport.screenshotCalls.length;await flush();expect(transport.screenshotCalls).toHaveLength(captured)
 await wrapper.get('[data-testid="desktop-workspace"]').trigger('click');await flush();expect(tab.previewVisible).toBe(true);expect(transport.instanceClear).not.toHaveBeenCalled()
 expect(wrapper.find('.desktop-sidebar .desktop-preview img').exists()).toBe(true)
 controller.setPreviewVisible(tab,false);await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await wrapper.get('[data-testid="desktop-workspace"]').trigger('click');await flush();expect(tab.previewVisible).toBe(false)
})
it('uses one xterm renderer without snapshot polling and releases native control on tab switch',async()=>{
 const {wrapper,controller,transport,terms,attach}=await setup();const first=await attach()
 expect(wrapper.find('[data-testid="open-desktop-history"]').exists()).toBe(false)
 expect(transport.runtimeHistory).toHaveBeenCalledWith(first.sessionId,{op:'terminalMouse',enabled:true});expect(terms[0].disposed).toBe(false)
 expect(wrapper.find('.desktop-history').exists()).toBe(false)
 expect(transport.runtimeHistory.mock.calls.some(([,r])=>r.op==='viewport')).toBe(false)
 const second=makeTarget('other','host');controller.openTarget({dto:makeHost('host',[second]),expanded:true},second,false);await flush()
 expect(transport.runtimeHistory).toHaveBeenCalledWith(first.sessionId,{op:'close'});expect(terms[0].disposed).toBe(false)
})
it('closes only the finished Shell and leaves genuine transport failures as recoverable connection state',async()=>{
 const {controller,transport,attach}=await setup();const first=await attach();const secondTarget=makeTarget('other','host');controller.openTarget({dto:makeHost('host',[secondTarget]),expanded:true},secondTarget,false);await flush();const second=controller.activeTab.value!
 transport.feed(first.sessionId!,{bytes:[],exitCode:null,runtime:{batchId:0,localGeneration:1,events:[],ended:'exited'}});await flush();expect(controller.tabs).toHaveLength(1);expect(controller.tabs[0]).toBe(second)
 const gate=deferred<void>();transport.runtimeRecover=vi.fn(()=>gate.promise)
 transport.feed(second.sessionId!,{bytes:[],exitCode:null,runtime:{batchId:0,localGeneration:1,events:[],ended:'transport'}});await flush();expect(controller.tabs).toHaveLength(1);expect(second.status).toBe('connecting');gate.resolve();await flush()
})
it('does not steal form keyboard shortcuts or detach healthy sessions when document hides',async()=>{
 const {wrapper,controller,transport,terms,attach}=await setup();await attach();await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush()
 await wrapper.get('[data-testid="settings-screens"]').trigger('click');await flush()
 const before=controller.activeKey.value;const input=wrapper.get('[data-testid="screen-name"]');await input.trigger('keydown',{code:'PageDown',ctrlKey:true});expect(controller.activeKey.value).toBe(before)
 const descriptor=Object.getOwnPropertyDescriptor(document,'hidden');Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));await flush();expect(transport.closeCalls).toEqual([]);expect(terms[0].disposed).toBe(false)
 Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'));await flush();if(descriptor)Object.defineProperty(document,'hidden',descriptor)
})

it('does not leave settings when the background session status or recovery changes',async()=>{
 const {wrapper,attach}=await setup();const tab=await attach();await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush()
 tab.status='connecting';await flush();tab.status='live';await flush()
 expect(wrapper.get('.desktop-settings-page').isVisible()).toBe(true)
 expect(wrapper.get('.desktop-workspace').isVisible()).toBe(false)
})

it('uses inline shared connection settings and clears secret drafts when switching categories', async () => {
 const { wrapper } = await setup()
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="settings-connections"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="name"]').setValue('Unsaved draft')
 await wrapper.get('[data-testid="credential"]').setValue('fixture-draft-secret')
 expect(wrapper.find('[data-testid="desktop-manage-hosts"]').exists()).toBe(false)
 expect(wrapper.find('.runtime-host-backdrop:not(.embedded-host-page)').exists()).toBe(false)
 expect(wrapper.find('[data-host-id="host"]').exists()).toBe(true)
 expect(document.querySelectorAll('[data-testid="name"]')).toHaveLength(1)
 expect(document.querySelectorAll('[data-testid="credential"]')).toHaveLength(1)
 expect(document.querySelectorAll('#runtime-host-heading')).toHaveLength(1)
 await wrapper.get('[data-testid="settings-appearance"]').trigger('click'); await flush()
 expect(wrapper.find('[data-testid="credential"]').exists()).toBe(false)
 await wrapper.get('[data-testid="settings-connections"]').trigger('click'); await flush()
 expect(wrapper.find('.embedded-settings-page').exists()).toBe(true)
 expect(document.querySelectorAll('[data-testid="name"]')).toHaveLength(1)
 expect(document.querySelectorAll('[data-testid="credential"]')).toHaveLength(1)
 expect((wrapper.get('[data-testid="name"]').element as HTMLInputElement).value).toBe('')
 expect((wrapper.get('[data-testid="credential"]').element as HTMLInputElement).value).toBe('')
 await wrapper.get('[data-testid="name"]').setValue('New draft')
 expect((wrapper.get('[data-testid="name"]').element as HTMLInputElement).value).toBe('New draft')
})

it('enables a chosen Screen once without picker-induced disable/enable ping-pong', async () => {
 const { wrapper, transport, attach } = await setup(); const tab = await attach()
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="choose-screen-screen-A"]').trigger('click'); await flush()
 expect(transport.desktopCalls).toEqual([{ sid: tab.sessionId, enabled: true }])
 expect(tab.previewVisible).toBe(true); expect(tab.instanceDesktop?.id).toBe('screen-A'); expect(tab.shot).not.toBeNull()
})

it('shows the newly selected Screen after manually hiding the previous preview', async () => {
 const { wrapper, transport, attach } = await setup(); const tab = await attach()
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="choose-screen-screen-A"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="preview-visibility"]').trigger('click'); await flush()
 expect(tab.previewVisible).toBe(false)
 const before = transport.desktopCalls.length
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="choose-screen-screen-B"]').trigger('click'); await flush()
 expect(tab.previewVisible).toBe(true); expect(tab.instanceDesktop?.id).toBe('screen-B'); expect(tab.shot).not.toBeNull()
 expect(transport.desktopCalls.slice(before)).toEqual([{ sid: tab.sessionId, enabled: true }])
})

it('restores an explicit new Screen selection after leaving Connect while selection is pending', async () => {
 const { wrapper, transport, attach } = await setup(); const tab = await attach()
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="choose-screen-screen-A"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="preview-visibility"]').trigger('click'); await flush()
 const pending = deferred<void>()
 const select = transport.instanceSelect.getMockImplementation()!
 transport.instanceSelect.mockImplementationOnce(async (sid, id) => { await pending.promise; return select(sid, id) })
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="choose-screen-screen-B"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click'); await flush()
 pending.resolve(); await flush()
 expect(tab.previewVisible).toBe(false); expect(tab.shot).toBeNull()
 expect(tab.instanceDesktop?.id).toBe('screen-B')
 await wrapper.get('[data-testid="desktop-workspace"]').trigger('click'); await flush()
 expect(tab.previewVisible).toBe(true); expect(tab.instanceDesktop?.id).toBe('screen-B'); expect(tab.shot).not.toBeNull()
})

it('keeps a native import picker mounted and blocks navigation until it returns', async () => {
 const { wrapper, transport } = await setup()
 const pending = deferred<Awaited<ReturnType<typeof transport.transferPickImport>>>()
 transport.transferPickImport = vi.fn(() => pending.promise)
 transport.transferCancel = vi.fn(async () => ({ status: 'cancelled' as const }))
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="settings-connections"]').trigger('click')
 await flush()
 const button = wrapper.findAll('button').find(node => node.text() === '匯入連線')!
 await button.trigger('click'); await flush()
 await wrapper.get('[data-testid="transfer-pick"]').trigger('click'); await flush()
 expect(transport.transferPickImport).toHaveBeenCalledTimes(1)
 const descriptor = Object.getOwnPropertyDescriptor(document, 'hidden')
 try {
  Object.defineProperty(document, 'hidden', { configurable: true, value: true })
  document.dispatchEvent(new Event('visibilitychange')); await flush()
  expect(wrapper.find('[data-testid="runtime-transfer"]').exists()).toBe(true)
  expect(transport.transferCancel).not.toHaveBeenCalled()
 } finally {
  Object.defineProperty(document, 'hidden', { configurable: true, value: false })
  document.dispatchEvent(new Event('visibilitychange')); await flush()
  pending.resolve({ status: 'cancelled', token: null, entries: [] }); await flush()
  if (descriptor) Object.defineProperty(document, 'hidden', descriptor)
 }
})

it('closes the Screen picker immediately and shows loading while CLI remains active', async () => {
 const { wrapper, transport, terms, attach } = await setup(); await attach()
 const pending = deferred<Awaited<ReturnType<typeof transport.instanceSelect>>>()
 transport.instanceSelect.mockImplementationOnce(() => pending.promise)
 await wrapper.get('[data-testid="desktop-toggle"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="choose-screen-screen-A"]').trigger('click'); await flush()
 expect(wrapper.find('[data-testid="screen-picker"]').exists()).toBe(false)
 expect(wrapper.get('.preview-loading').text()).toContain('正在載入桌面')
 expect(wrapper.get('.desktop-connect-page .pane').isVisible()).toBe(true)
 expect(terms[0].focusCount).toBeGreaterThan(1)
 transport.shot = { ...transport.shot, desktopId: screen.id, desktopRevision: screen.revision }
 pending.resolve({ desktop: screen, toolReady: false }); await flush()
 expect(wrapper.find('.preview-loading').exists()).toBe(false)
 expect(wrapper.get('[data-testid="desktop-toggle"]').text()).toBe('')
 expect(wrapper.get('.preview .p-title').text()).toBe('Screen A')
})
