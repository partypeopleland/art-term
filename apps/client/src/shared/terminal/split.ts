import type { TabState } from '../state/controller-types'

export type SplitDirection = 'up' | 'down' | 'left' | 'right'
export type SplitAxis = 'row' | 'column'

/**
 * Pane tree for the Windows split view. Every `conn` leaf owns a full
 * TabState — its own target, session, poll loop and input queue — so panes
 * are independent connections, not mirrors of one screen. `mirror` leaves
 * stay read-only views of an existing connection (issue #34 behaviour).
 * `empty` leaves wait for the user to pick or create a session.
 */
export type PaneLeaf =
  | { kind: 'conn'; id: string; tab: TabState }
  | { kind: 'mirror'; id: string; of: TabState }
  | { kind: 'empty'; id: string }

export interface PaneSplitNode {
  kind: 'split'
  id: string
  axis: SplitAxis
  /** Fraction of the primary side along the split axis, clamped 0.15–0.85. */
  ratio: number
  a: PaneNode
  b: PaneNode
}

export type PaneNode = PaneLeaf | PaneSplitNode

export function splitAxis(direction: SplitDirection): SplitAxis {
  return direction === 'left' || direction === 'right' ? 'row' : 'column'
}

/** Wrap `inner` and `added` in a split node; `added` lands on the side the direction names. */
export function wrapSplit(direction: SplitDirection, inner: PaneNode, added: PaneNode, id: string): PaneSplitNode {
  const axis = splitAxis(direction)
  const a = direction === 'left' || direction === 'up' ? added : inner
  const b = direction === 'left' || direction === 'up' ? inner : added
  return { kind: 'split', id, axis, ratio: 0.5, a, b }
}

function isLeaf(node: PaneNode): node is PaneLeaf {
  return node.kind !== 'split'
}

/** Replace the leaf `leafId` with `wrapSplit(direction, leaf, added)`; returns the same root when absent. */
export function splitPaneLeaf(
  root: PaneNode,
  leafId: string,
  direction: SplitDirection,
  added: PaneNode,
  splitId: string
): PaneNode {
  if (isLeaf(root)) {
    return root.id === leafId ? wrapSplit(direction, root, added, splitId) : root
  }
  const a = splitPaneLeaf(root.a, leafId, direction, added, splitId)
  if (a !== root.a) return { ...root, a }
  const b = splitPaneLeaf(root.b, leafId, direction, added, splitId)
  if (b !== root.b) return { ...root, b }
  return root
}

/**
 * Remove the leaf `leafId`, collapsing split nodes that lose a child.
 * Returns null when nothing remains.
 */
export function removePaneLeaf(root: PaneNode, leafId: string): PaneNode | null {
  if (isLeaf(root)) return root.id === leafId ? null : root
  const a = removePaneLeaf(root.a, leafId)
  const b = a === root.a ? removePaneLeaf(root.b, leafId) : root.b
  if (a === null) return b
  if (b === null) return a
  if (a === root.a && b === root.b) return root
  return { ...root, a, b }
}

/** Replace the leaf `leafId` with `node`; returns the same root when absent. */
export function replacePaneLeaf(root: PaneNode, leafId: string, node: PaneNode): PaneNode {
  if (isLeaf(root)) return root.id === leafId ? node : root
  const a = replacePaneLeaf(root.a, leafId, node)
  if (a !== root.a) return { ...root, a }
  const b = replacePaneLeaf(root.b, leafId, node)
  if (b !== root.b) return { ...root, b }
  return root
}

/** All leaves in display order. */
export function paneLeaves(root: PaneNode | null): PaneLeaf[] {
  if (!root) return []
  if (root.kind !== 'split') return [root]
  return [...paneLeaves(root.a), ...paneLeaves(root.b)]
}

/** Connection panes in display order (left→right, top→bottom). */
export function paneConnTabs(root: PaneNode | null): TabState[] {
  if (!root) return []
  if (root.kind === 'conn') return [root.tab]
  if (root.kind !== 'split') return []
  return [...paneConnTabs(root.a), ...paneConnTabs(root.b)]
}

/** Find a leaf by id; returns the leaf object or null. */
export function findPaneLeaf(root: PaneNode | null, leafId: string): PaneLeaf | null {
  if (!root) return null
  if (root.kind !== 'split') return root.id === leafId ? root : null
  return findPaneLeaf(root.a, leafId) ?? findPaneLeaf(root.b, leafId)
}

export const PANE_RATIO_MIN = 0.15
export const PANE_RATIO_MAX = 0.85

export function clampPaneRatio(ratio: number): number {
  return Math.min(PANE_RATIO_MAX, Math.max(PANE_RATIO_MIN, ratio))
}
