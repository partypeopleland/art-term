export type SplitDirection = 'up' | 'down' | 'left' | 'right'
export type SplitAxis = 'row' | 'column'
export type SplitPane = 'primary' | 'divider' | 'mirror'

export function splitLayout(direction: SplitDirection): { axis: SplitAxis; mirrorFirst: boolean } {
  switch (direction) {
    case 'up':
      return { axis: 'column', mirrorFirst: true }
    case 'down':
      return { axis: 'column', mirrorFirst: false }
    case 'left':
      return { axis: 'row', mirrorFirst: true }
    case 'right':
      return { axis: 'row', mirrorFirst: false }
  }
}

export function splitPaneOrder(direction: SplitDirection): SplitPane[] {
  return splitLayout(direction).mirrorFirst
    ? ['mirror', 'divider', 'primary']
    : ['primary', 'divider', 'mirror']
}
