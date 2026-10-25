import { describe, expect, it } from 'vitest'
import { clampPan, panelImageSize, zoomAt } from '../src/shared/components/preview-geometry'

describe('desktop preview zoom geometry', () => {
  it('uses the source aspect ratio and shrinks both dimensions within the available space', () => {
    expect(panelImageSize({ width: 1920, height: 1080 }, { width: 900, height: 900 })).toEqual({ width: 640, height: 360 })
    expect(panelImageSize({ width: 1080, height: 1920 }, { width: 900, height: 900 })).toEqual({ width: 360, height: 640 })
    expect(panelImageSize({ width: 2000, height: 500 }, { width: 300, height: 200 })).toEqual({ width: 300, height: 75 })
  })
  it('anchors zoom to the pointer, clamps pan to image bounds, and centers at 1×', () => {
    const image = { width: 1600, height: 900 }, viewport = { width: 640, height: 360 }
    expect(zoomAt({ x: 0, y: 0 }, 1, 2, { x: 120, y: 50 }, image, viewport)).toEqual({ x: -120, y: -50 })
    expect(clampPan({ x: 999, y: -999 }, 2, image, viewport)).toEqual({ x: 320, y: -180 })
    expect(clampPan({ x: 80, y: -40 }, 1, image, viewport)).toEqual({ x: 0, y: 0 })
  })
})
