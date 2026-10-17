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
it('keeps terminal mounted and draining under non-toggle full settings, Main and homepage; restores fit/focus',async()=>{
 const {wrapper,controller,transport,terms,attach}=await setup();const tab=await attach();const sid=tab.sessionId!;const term=terms[0]
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush();const focus=term.focusCount
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await flush()
 expect(wrapper.find('.desktop-settings-page').exists()).toBe(true);expect(terms).toHaveLength(1);expect(term.disposed).toBe(false)
 transport.feed(sid,{bytes:[72,105],exitCode:null});await flush();expect(term.text()).toBe('Hi');expect(term.focusCount).toBe(focus)
 expect(wrapper.find('[data-testid="desktop-manage-hosts"]').exists()).toBe(true)
 expect(wrapper.find('[data-testid="screen-settings"]').exists()).toBe(true)
 await wrapper.get('[aria-label="軟體首頁"]').trigger('click');await flush();expect(wrapper.get('.version-value').text()).toContain('Windows');expect(wrapper.get('.version-value').text()).not.toContain('Android')
 await wrapper.get('[data-testid="desktop-main"]').trigger('click');await flush();expect(transport.closeCalls).toEqual([])
 await wrapper.get('[data-testid="desktop-connect"]').trigger('click');await flush();expect(term.focusCount).toBeGreaterThan(focus);expect(term.fitCount).toBeGreaterThan(1);expect(controller.tabs[0]).toBe(tab)
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
it('stops all previews away from Connect, restores prior visibility and never clears tools',async()=>{
 const {wrapper,controller,transport,attach}=await setup();const tab=await attach();await controller.selectInstance(tab,screen);await flush();expect(tab.previewVisible).toBe(true)
 await wrapper.get('[data-testid="desktop-main"]').trigger('click');await flush();expect(tab.previewVisible).toBe(false);expect(tab.instanceDesktop?.id).toBe(screen.id)
 const captured=transport.screenshotCalls.length;await flush();expect(transport.screenshotCalls).toHaveLength(captured)
 await wrapper.get('[data-testid="desktop-connect"]').trigger('click');await flush();expect(tab.previewVisible).toBe(true);expect(transport.instanceClear).not.toHaveBeenCalled()
 controller.setPreviewVisible(tab,false);await wrapper.get('[data-testid="desktop-settings"]').trigger('click');await wrapper.get('[data-testid="desktop-connect"]').trigger('click');await flush();expect(tab.previewVisible).toBe(false)
})
it('binds history to one tab/session, loads only on explicit open, closes on tab switch and keeps terminal alive',async()=>{
 const {wrapper,controller,transport,terms,attach}=await setup();const first=await attach();expect(transport.runtimeHistory).not.toHaveBeenCalled()
 await wrapper.get('[data-testid="open-desktop-history"]').trigger('click');await flush();expect(transport.runtimeHistory).toHaveBeenCalledWith(first.sessionId,{op:'viewport'});expect(terms[0].disposed).toBe(false)
 const second=makeTarget('other','host');controller.openTarget({dto:makeHost('host',[second]),expanded:true},second,false);await flush();expect(wrapper.find('[data-testid="desktop-history"]').exists()).toBe(false)
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
 const before=controller.activeKey.value;const input=wrapper.get('[data-testid="screen-name"]');await input.trigger('keydown',{code:'PageDown',ctrlKey:true});expect(controller.activeKey.value).toBe(before)
 const descriptor=Object.getOwnPropertyDescriptor(document,'hidden');Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));await flush();expect(transport.closeCalls).toEqual([]);expect(terms[0].disposed).toBe(false)
 Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'));await flush();if(descriptor)Object.defineProperty(document,'hidden',descriptor)
})

it('does not navigate away from Main when the background session status or recovery changes',async()=>{
 const {wrapper,controller,attach}=await setup();const tab=await attach();await wrapper.get('[data-testid="desktop-main"]').trigger('click');await flush()
 tab.status='connecting';await flush();tab.status='live';await flush()
 expect(wrapper.get('[data-testid="desktop-main"]').attributes('aria-current')).toBe('page')
 expect(wrapper.get('[data-testid="desktop-connect"]').attributes('aria-current')).toBeUndefined()
})

it('mounts only one host editor when management covers settings and clears the hidden draft', async () => {
 const { wrapper } = await setup()
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click'); await flush()
 await wrapper.get('[data-testid="name"]').setValue('Unsaved draft')
 await wrapper.get('[data-testid="credential"]').setValue('fixture-draft-secret')
 await wrapper.get('[data-testid="desktop-manage-hosts"]').trigger('click'); await flush()
 expect(wrapper.find('.embedded-settings-page').exists()).toBe(false)
 const add = wrapper.findAll('button').find(button => button.text() === '新增 openab-pty 連線')!
 await add.trigger('click'); await flush()
 expect(document.querySelectorAll('[data-testid="name"]')).toHaveLength(1)
 expect(document.querySelectorAll('[data-testid="credential"]')).toHaveLength(1)
 expect(document.querySelectorAll('#runtime-host-heading')).toHaveLength(1)
 const close = wrapper.findAll('button').find(button => button.text() === '關閉')!
 await close.trigger('click'); await flush()
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
 await wrapper.get('[data-testid="desktop-main"]').trigger('click'); await flush()
 pending.resolve(); await flush()
 expect(tab.previewVisible).toBe(false); expect(tab.shot).toBeNull()
 expect(tab.instanceDesktop?.id).toBe('screen-B')
 await wrapper.get('[data-testid="desktop-connect"]').trigger('click'); await flush()
 expect(tab.previewVisible).toBe(true); expect(tab.instanceDesktop?.id).toBe('screen-B'); expect(tab.shot).not.toBeNull()
})

it.each([true, false])('keeps a native import picker mounted during visibility changes (management=%s)', async management => {
 const { wrapper, transport } = await setup()
 const pending = deferred<Awaited<ReturnType<typeof transport.transferPickImport>>>()
 transport.transferPickImport = vi.fn(() => pending.promise)
 transport.transferCancel = vi.fn(async () => ({ status: 'cancelled' as const }))
 await wrapper.get('[data-testid="desktop-settings"]').trigger('click'); await flush()
 if (management) await wrapper.get('[data-testid="desktop-manage-hosts"]').trigger('click')
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
 if (management) {
  const close = wrapper.findAll('.runtime-hosts > header button').find(node => node.text() === '關閉')!
  await close.trigger('click'); await flush()
  await wrapper.get('[data-testid="desktop-manage-hosts"]').trigger('click'); await flush()
  Object.defineProperty(document, 'hidden', { configurable: true, value: true })
  document.dispatchEvent(new Event('visibilitychange')); await flush()
  expect(wrapper.find('.runtime-host-backdrop:not(.embedded-host-page)').exists()).toBe(false)
  Object.defineProperty(document, 'hidden', { configurable: true, value: false })
  document.dispatchEvent(new Event('visibilitychange')); await flush()
  if (descriptor) Object.defineProperty(document, 'hidden', descriptor)
 }
})
