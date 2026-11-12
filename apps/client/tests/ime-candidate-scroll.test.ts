// @vitest-environment node
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

// Issue #14: the Windows IME candidate window must not shift the terminal
// canvas. Chromium scrolls overflow:hidden ancestors to keep the IME window
// visible; the terminal host and workspace body must be non-scrollable
// (overflow: clip) so composition cannot move the rendered terminal.
const css = readFileSync(resolve(process.cwd(), 'src/shared/style.css'), 'utf8')

function declarations(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))
  expect(match, `rule for ${selector}`).not.toBeNull()
  return match![1]
}

describe('IME candidate window scroll isolation (issue 14)', () => {
  it('.term-host is not a programmatic scroll container', () => {
    expect(declarations('.term-host')).toMatch(/overflow:\s*clip/)
  })

  it('workspace body is not a programmatic scroll container', () => {
    expect(declarations('.windows-app .workspace-body')).toMatch(/overflow:\s*clip/)
  })
})
