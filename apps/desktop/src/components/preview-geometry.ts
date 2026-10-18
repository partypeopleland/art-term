export interface Size { width: number; height: number }
export interface Point { x: number; y: number }

export function fittedImage(image: Size, viewport: Size): Size {
  if (!image.width || !image.height || !viewport.width || !viewport.height) return { width: 0, height: 0 }
  const ratio = Math.min(viewport.width / image.width, viewport.height / image.height)
  return { width: image.width * ratio, height: image.height * ratio }
}

export function clampPan(pan: Point, zoom: number, image: Size, viewport: Size): Point {
  const fit = fittedImage(image, viewport)
  const maxX = Math.max(0, (fit.width * zoom - viewport.width) / 2)
  const maxY = Math.max(0, (fit.height * zoom - viewport.height) / 2)
  return { x: maxX ? Math.max(-maxX, Math.min(maxX, pan.x)) : 0, y: maxY ? Math.max(-maxY, Math.min(maxY, pan.y)) : 0 }
}

export function zoomAt(pan: Point, previous: number, next: number, anchor: Point, image: Size, viewport: Size): Point {
  const factor = next / previous
  return clampPan({ x: anchor.x - (anchor.x - pan.x) * factor, y: anchor.y - (anchor.y - pan.y) * factor }, next, image, viewport)
}

export function panelImageSize(image: Size, available: Size): Size {
  if (!image.width || !image.height) return { width: 0, height: 0 }
  const scale = Math.min(640 / Math.max(image.width, image.height), available.width / image.width, available.height / image.height)
  return { width: image.width * Math.max(0, scale), height: image.height * Math.max(0, scale) }
}
