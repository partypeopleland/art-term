/** Private build input only. Public builds substitute null for this constant. */
declare const __ART_TERM_TEST_COMPUTER_PREFILL__: unknown

export interface TestComputerPrefill {
  name: string
  baseUrl: string
  credential: string
}

export function parseTestComputerPrefill(value: unknown): TestComputerPrefill | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const { name, baseUrl, credential } = value as Partial<TestComputerPrefill>
  if (typeof name !== 'string' || typeof baseUrl !== 'string' || typeof credential !== 'string') return null
  const bytes = (text: string) => new TextEncoder().encode(text).length
  if (!name || name.trim() !== name || bytes(name) > 128 || /[\u0000-\u001f\u007f]/.test(name)) return null
  if (!credential || bytes(credential) > 1024 || /[\s\u0000-\u001f\u007f]/.test(credential)) return null
  if (bytes(baseUrl) > 2048 || /[\\\u0000-\u0020\u007f]/.test(baseUrl)) return null
  const shape = /^https:\/\/([^/?#]+)\/?$/i.exec(baseUrl)
  if (!shape || shape[1].includes('@')) return null
  try {
    const url = new URL(baseUrl)
    if (url.protocol !== 'https:' || !url.hostname || url.username || url.password || url.pathname !== '/' || url.search || url.hash) return null
  } catch { return null }
  return { name, baseUrl, credential }
}

export function getTestComputerPrefill(): TestComputerPrefill | null {
  return parseTestComputerPrefill(__ART_TERM_TEST_COMPUTER_PREFILL__)
}
