import type { InjectionKey } from 'vue'
import type { NativeTerminalLine } from './native-buffer'
export interface NativeHistorySnapshot {
  readerId: string
  records: NativeTerminalLine[]
  visible: boolean
  bounds: { x: number; y: number; width: number; height: number; viewportWidth: number }
}
export interface NativeHistoryEvent {
  readerId: string
  kind: 'tap' | 'visibility'
  visible?: boolean
}
export interface NativeHistoryReader {
  open(snapshot: NativeHistorySnapshot, receive: (event: NativeHistoryEvent) => void): Promise<void>
  update(snapshot: NativeHistorySnapshot): Promise<void>
  close(readerId: string): Promise<void>
}
export const NativeHistoryKey: InjectionKey<() => NativeHistoryReader> = Symbol('NativeTerminalReader')
