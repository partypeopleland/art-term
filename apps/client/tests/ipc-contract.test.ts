// @vitest-environment node
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it, vi } from 'vitest'
import { createTauriTransport } from '../src/desktop/tauri'
import { createMobileTauriTransport } from '../src/mobile/tauri'
import type { Transport, TransferChoice } from '../src/shared/transport/types'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

function source(path: string): string {
  return readFileSync(resolve(process.cwd(), '../../crates/art-term-gui', path), 'utf8')
}

async function exercise(transport: Transport): Promise<string[]> {
  invoke.mockClear()
  await transport.transferExport({selected:[{id:'fixture',expectedRevision:1}],includeCredentials:false,password:''})
  await transport.transferPickImport()
  await transport.transferUnlock('opaque-token','fictional password')
  await transport.transferApply('opaque-token',[{key:'opaque-entry',action:'skip'}])
  await transport.transferCancel()
  await transport.runtimeHosts()
  expect(transport.instanceRestore).toBeTypeOf('function')
  await transport.instanceRestore!('issued-session')
  await transport.open('issued-target', 24, 80)
  await transport.poll('issued-session')
  await transport.write('issued-session', [0x03])
  await transport.resize('issued-session', 39, 100)
  await transport.desktop('issued-session', true)
  await transport.screenshot('issued-session')
  await transport.close('issued-session')
  return invoke.mock.calls.map(([command]) => command as string)
}

describe('adapters agree with the registered and authorized native surface', () => {
  it.each([createTauriTransport, createMobileTauriTransport])('restores the selected Screen through the native command', async (create) => {
    const selection = { desktop: { id: 'screen-a', revision: 1, name: 'Screen A', mcpUrl: 'https://screen.example.test/mcp', tokenPresent: true }, toolReady: true }
    invoke.mockReset().mockResolvedValueOnce(selection)
    const transport = create()
    expect(transport.instanceRestore).toBeTypeOf('function')
    await expect(transport.instanceRestore!('issued-session')).resolves.toEqual(selection)
    expect(invoke).toHaveBeenCalledExactlyOnceWith('gui_instance_restore', { sessionId: 'issued-session' })
    invoke.mockResolvedValue(undefined)
  })

  it.each([createTauriTransport, createMobileTauriTransport])('snapshots supplemental credential arguments before the UI clears them', async (create) => {
    invoke.mockClear()
    const choices: TransferChoice[] = [{key:'opaque',action:'add',credential:'fictional-one-shot'}]
    const pending = create().transferApply('preview', choices)
    choices[0].credential = ''
    expect(invoke.mock.calls[0]).toEqual(['gui_transfer_apply', {token:'preview',choices:[{key:'opaque',action:'add',credential:'fictional-one-shot'}]}])
    await pending
  })

  it.each([
    ['Windows', createTauriTransport, 'src/desktop/main.rs', 'permissions/gui.toml'],
    ['Android', createMobileTauriTransport, 'src/platform/android/mobile.rs', 'permissions/mobile.toml']
  ] as const)('%s has no callable command missing from its handler or ACL', async (_, create, native, acl) => {
    const handlers = source(native).match(/generate_handler!\[([\s\S]*?)\]/)?.[1]
    const allowed = source(acl).match(/commands\.allow\s*=\s*\[([\s\S]*?)\]/)?.[1]
    expect(handlers).toBeDefined()
    expect(allowed).toBeDefined()
    const registered = new Set(
      handlers!
        .split(',')
        .map((name) => name.trim().split('::').pop()!)
    )
    const authorized = new Set([...allowed!.matchAll(/"([^"]+)"/g)].map((match) => match[1]))
    for (const command of await exercise(create())) {
      expect(registered.has(command), `${command} must be registered`).toBe(true)
      expect(authorized.has(command), `${command} must be authorized`).toBe(true)
    }
  })
})

it.each([[createTauriTransport, 'gui_write'], [createMobileTauriTransport, 'mobile_write']] as const)('only adds the terminal reply source flag for protocol data', async (create, command) => {
  invoke.mockClear()
  await create().write('issued-session', [65])
  expect(invoke.mock.calls[0]).toEqual([command, { sessionId: 'issued-session', bytes: [65] }])
  await create().write('issued-session', [27, 91, 82], true)
  expect(invoke.mock.calls[1]).toEqual([command, { sessionId: 'issued-session', bytes: [27, 91, 82], terminalReply: true }])
})
