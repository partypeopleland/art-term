import { afterEach, expect, it, vi } from 'vitest'
import { Terminal } from '@xterm/xterm'
import { attachTerminalLinks, terminalTextLinks } from '../src/terminal/links'
window.matchMedia = (() => ({ matches:false,addListener(){},removeListener(){},addEventListener(){},removeEventListener(){} })) as unknown as typeof window.matchMedia
const cleanups:(()=>void)[]=[]
afterEach(()=>{cleanups.splice(0).forEach(f=>f());document.body.replaceChildren();vi.useRealTimers()})
async function setup(text='中文 https://example.com/a?q=1。',cols=80){
 const host=document.createElement('div');document.body.append(host)
 const term=new Terminal({cols,rows:5});term.open(host)
 const screen=host.querySelector('.xterm-screen')!
 vi.spyOn(screen,'getBoundingClientRect').mockReturnValue({left:0,top:0,width:cols*10,height:100,right:cols*10,bottom:100} as DOMRect)
 const open=vi.fn(async (_url:string)=>{}),input=vi.fn();term.onData(input)
 const links=attachTerminalLinks(host,term,open);cleanups.push(()=>{links.dispose();term.dispose()})
 await new Promise<void>(resolve=>term.write(text,resolve))
 const mouse=(type:string,x=60,y=10)=>screen.dispatchEvent(new MouseEvent(type,{clientX:x,clientY:y,button:0,bubbles:true,cancelable:true}))
 const touch=(type:string,x=60,y=10)=>{const e=new Event(type,{bubbles:true,cancelable:true});Object.defineProperties(e,{touches:{value:type==='touchend'?[]:[{clientX:x,clientY:y}]},changedTouches:{value:[{clientX:x,clientY:y}]}});screen.dispatchEvent(e)}
 return {host,term,open,input,mouse,touch}
}
it('opens the original wide-character HTTP URL synchronously once without clipboard notice or PTY input',async()=>{
 const s=await setup();s.mouse('mousedown');s.mouse('mouseup');s.mouse('click')
 expect(s.open).toHaveBeenCalledExactlyOnceWith('https://example.com/a?q=1')
 await Promise.resolve();await Promise.resolve()
 expect((s.host.querySelector('.terminal-link-notice') as HTMLElement).hidden).toBe(true)
 expect(s.input).not.toHaveBeenCalled()
})
it('finds soft-wrapped links and opens a short touch once',async()=>{
 const s=await setup('中文 https://example.com/wrapped/path',20)
 s.touch('touchstart',30,30);s.touch('touchend',30,30)
 expect(s.open).toHaveBeenCalledExactlyOnceWith('https://example.com/wrapped/path')
 expect(s.input).not.toHaveBeenCalled()
})
it('cancels drag, swipe and long press',async()=>{
 const s=await setup();s.mouse('mousedown');s.mouse('mousemove',100);s.mouse('mousemove');s.mouse('mouseup')
 s.touch('touchstart');s.touch('touchmove',100);s.touch('touchend',100)
 vi.useFakeTimers();s.touch('touchstart');vi.advanceTimersByTime(700);s.touch('touchend')
 expect(s.open).not.toHaveBeenCalled()
})
it('rejects arbitrary OSC8 schemes/control/oversized URLs at the actual callback boundary',async()=>{
 const s=await setup();const event=new MouseEvent('click',{cancelable:true})
 for(const url of ['file:///secret','javascript:alert(1)','data:text/plain,x','https://host/\nattack','https://host/'+ 'x'.repeat(8192), 'https://host/'+'界'.repeat(3000)]) s.term.options.linkHandler!.activate(event,url,{start:{x:1,y:1},end:{x:2,y:1}})
 expect(s.open).not.toHaveBeenCalled()
})
it('shows a failure only when browser opening fails',async()=>{
 const s=await setup('http://example.com/');s.open.mockRejectedValueOnce(new Error('no browser'))
 s.mouse('mousedown',20);s.mouse('mouseup',20)
 await Promise.resolve();await Promise.resolve();await Promise.resolve()
 expect(s.host.querySelector('.terminal-link-notice')?.textContent).toContain('開啟失敗')
 expect(s.input).not.toHaveBeenCalled()
})
it('retains punctuation and balanced URL matching',()=>{
 expect(terminalTextLinks('(http://example.com/a_(b)). https://example.com/z!').map(x=>x.text)).toEqual(['http://example.com/a_(b)','https://example.com/z'])
})
