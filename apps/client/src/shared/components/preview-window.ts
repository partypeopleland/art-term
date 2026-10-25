import { panelImageSize, type Point, type Size } from './preview-geometry'

export interface PanelRect extends Point, Size {}
export type ResizeEdge = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw'
export const resizeEdges: ResizeEdge[] = ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw']

export function defaultPanelSize(source: Size, area: Size): Size {
  const image = panelImageSize(source, { width: Math.max(0, area.width - 18), height: Math.max(0, area.height - 58) })
  return { width: (image.width || 280) + 2, height: (image.height || 140) + 42 }
}

export function boundPanel(rect: PanelRect, minimum: Size, area: Size): PanelRect {
  const width = Math.min(area.width, Math.max(minimum.width, rect.width))
  const height = Math.min(area.height, Math.max(minimum.height, rect.height))
  return { width, height, x: Math.max(0, Math.min(rect.x, area.width - width)), y: Math.max(0, Math.min(rect.y, area.height - height)) }
}

export function resizePanel(rect: PanelRect, edge: ResizeEdge, delta: Point, minimum: Size, area: Size): PanelRect {
  const minWidth = Math.min(minimum.width, area.width)
  const minHeight = Math.min(minimum.height, area.height)
  let { x, y, width, height } = rect
  if (edge.includes('e')) width = Math.max(minWidth, Math.min(area.width - x, width + delta.x))
  if (edge.includes('s')) height = Math.max(minHeight, Math.min(area.height - y, height + delta.y))
  if (edge.includes('w')) {
    x = Math.max(0, Math.min(rect.x + delta.x, rect.x + rect.width - minWidth))
    width = rect.x + rect.width - x
  }
  if (edge.includes('n')) {
    y = Math.max(0, Math.min(rect.y + delta.y, rect.y + rect.height - minHeight))
    height = rect.y + rect.height - y
  }
  return { x, y, width, height }
}