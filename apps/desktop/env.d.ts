/// <reference types="vite/client" />

declare module 'virtual:art-term-info' {
  const info: { version: string; androidBuild: number; releases: { version: string; content: string }[] }
  export default info
}
