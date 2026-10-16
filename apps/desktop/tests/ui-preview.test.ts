import { describe, expect, it } from 'vitest'
import { createPreviewTransport } from '../ui-preview/transport'

describe('browser UI preview transport', () => {
  it('keeps edits local to one preview and never returns the submitted credential', async () => {
    const preview = createPreviewTransport('empty')
    const saved = await preview.runtimeSaveHost({ name: 'Sketch', baseUrl: 'https://sketch.example.test', credential: 'fictional-secret', category: 'host' })
    expect(saved.credentialPresent).toBe(true)
    expect(JSON.stringify(await preview.runtimeHosts())).not.toContain('fictional-secret')
    expect(await createPreviewTransport('empty').runtimeHosts()).toEqual([])
    await expect(preview.runtimeDeleteHost(saved.id, saved.revision + 1)).rejects.toThrow('示範設定已變更')
    await preview.runtimeDeleteHost(saved.id, saved.revision)
    expect(await preview.runtimeHosts()).toEqual([])
  })

  it('supports the real controller session flow with canned output and detach', async () => {
    const preview = createPreviewTransport()
    const [host] = await preview.runtimeHosts()
    const group = await preview.runtimeSessions(host.id, host.revision)
    const session = await preview.open(group.targets[0].id, 24, 80)
    const first = await preview.poll(session.id)
    expect(new TextDecoder().decode(Uint8Array.from(first.bytes))).toContain('不會執行指令')
    await preview.write(session.id, [...new TextEncoder().encode('echo hello\r')])
    const next = await preview.poll(session.id)
    expect(new TextDecoder().decode(Uint8Array.from(next.bytes))).toContain('未執行')
    await preview.close(session.id)
    expect((await preview.poll(session.id)).exitCode).toBe(0)
  })

  it('provides empty and error states and cancels system file pickers', async () => {
    expect(await createPreviewTransport('empty').runtimeHosts()).toEqual([])
    await expect(createPreviewTransport('error').runtimeHosts()).rejects.toThrow('示範：無法讀取')
    const preview = createPreviewTransport()
    expect((await preview.transferPickImport()).status).toBe('cancelled')
    expect((await preview.transferExport({ selected: [], includeCredentials: false, password: '' })).status).toBe('cancelled')
  })
})
