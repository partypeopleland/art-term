<script setup lang="ts">
import { ref } from 'vue'
import type { AppController, TabState } from '../state/controller'
import type { PaneLeaf, PaneNode, PaneSplitNode, SplitDirection } from '../terminal/split'
import { clampPaneRatio } from '../terminal/split'
import TerminalPane from './TerminalPane.vue'

// Recursive split-view renderer. `tab` is the owning tab (its own conn leaf is
// the primary pane); every `conn` leaf carries an independent TabState.
const props = defineProps<{
  node: PaneNode
  tab: TabState
  controller: AppController
  visible: boolean
  onSelectionFinished?: (tab: TabState) => void
}>()

const emit = defineEmits<{ pickConnection: [leafId: string] }>()

const DIRECTIONS: { dir: SplitDirection; label: string }[] = [
  { dir: 'right', label: '向右分割' },
  { dir: 'down', label: '向下分割' },
  { dir: 'left', label: '向左分割' },
  { dir: 'up', label: '向上分割' }
]

const splitEl = ref<HTMLElement | null>(null)
const menu = ref<{ x: number; y: number; leafId: string; leaf: PaneLeaf } | null>(null)

function leafClosable(leaf: PaneLeaf | null): boolean {
  if (!leaf) return false
  if (leaf.kind !== 'conn') return true
  // The owning tab's own conn pane closes like any other once a split
  // view exists (a survivor is promoted); alone it stays tab-owned.
  return leaf.tab !== props.tab || !!props.tab.paneRoot
}

function openMenu(event: MouseEvent, leaf: PaneLeaf): void {
  event.preventDefault()
  menu.value = { x: event.clientX, y: event.clientY, leafId: leaf.id, leaf }
}

function closeMenu(): void {
  menu.value = null
}

function menuLeaf(): PaneLeaf | null {
  return menu.value?.leaf ?? null
}

function splitFromMenu(direction: SplitDirection): void {
  const leafId = menu.value?.leafId
  closeMenu()
  if (leafId) props.controller.splitPane(props.tab, leafId, direction)
}

function mirrorFromMenu(): void {
  const leafId = menu.value?.leafId
  closeMenu()
  if (leafId) props.controller.mirrorPane(props.tab, leafId, 'down')
}

function closeFromMenu(): void {
  const leafId = menu.value?.leafId
  closeMenu()
  if (leafId) props.controller.closePaneLeaf(props.tab, leafId)
}

function pickFromMenu(): void {
  const leafId = menu.value?.leafId
  closeMenu()
  if (leafId) emit('pickConnection', leafId)
}

function branchStyle(node: PaneSplitNode, first: boolean): { flex: string } {
  const ratio = first ? node.ratio : 1 - node.ratio
  return { flex: `${ratio} 1 0` }
}

function startDrag(event: PointerEvent, node: PaneSplitNode): void {
  event.preventDefault()
  const el = splitEl.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  const horizontal = node.axis === 'row'
  const size = horizontal ? rect.width : rect.height
  if (!size) return
  const origin = horizontal ? rect.left : rect.top
  const start = (horizontal ? event.clientX : event.clientY) - origin
  const originRatio = node.ratio
  const move = (ev: PointerEvent) => {
    const pos = (horizontal ? ev.clientX : ev.clientY) - origin
    node.ratio = clampPaneRatio(originRatio + (pos - start) / size)
  }
  const up = () => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    window.removeEventListener('pointercancel', up)
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
  window.addEventListener('pointercancel', up)
}
</script>

<template>
  <div v-if="node.kind === 'split'" ref="splitEl" class="pane-split" :class="node.axis === 'row' ? 'pane-row' : 'pane-col'">
    <div class="pane-branch" :style="branchStyle(node, true)">
      <PaneSplitView :node="node.a" :tab="tab" :controller="controller" :visible="visible" :on-selection-finished="onSelectionFinished" @pick-connection="leafId => emit('pickConnection', leafId)" />
    </div>
    <div
      class="pane-divider"
      :class="node.axis"
      role="separator"
      tabindex="0"
      :aria-orientation="node.axis === 'row' ? 'vertical' : 'horizontal'"
      aria-label="調整窗格大小"
      @pointerdown="startDrag($event, node)"
    />
    <div class="pane-branch" :style="branchStyle(node, false)">
      <PaneSplitView :node="node.b" :tab="tab" :controller="controller" :visible="visible" :on-selection-finished="onSelectionFinished" @pick-connection="leafId => emit('pickConnection', leafId)" />
    </div>
  </div>

  <div v-else class="pane-cell" :class="{ focused: visible && !!tab.paneRoot && tab.focusedLeafId === node.id }" @contextmenu="openMenu($event, node)" @pointerdown="node.kind === 'empty' && controller.focusPane(tab, node.id)">
    <TerminalPane
      v-if="node.kind === 'conn'"
      :tab="node.tab"
      :active="visible"
      :focused="tab.paneRoot ? visible && tab.focusedLeafId === node.id : undefined"
      :controller="controller"
      :reader-visible="visible"
      native-scroll
      :closable="leafClosable(node)"
      close-label="關閉窗格"
      :on-selection-finished="onSelectionFinished"
      @close="controller.closePaneLeaf(tab, node.id)"
      @activate="controller.focusPane(tab, node.id)"
    />
    <TerminalPane
      v-else-if="node.kind === 'mirror'"
      :tab="node.of"
      mirror
      :active="visible"
      :focused="false"
      :controller="controller"
      :reader-visible="false"
      closable
      close-label="關閉鏡像窗格"
      @close="controller.closePaneLeaf(tab, node.id)"
      @activate="controller.focusPane(tab, node.id)"
    />
    <div v-else class="pane-empty">
      <div class="pane-empty-box">
        <strong>未連線窗格</strong>
        <p class="muted">選擇既有 Shell 或建立新的連線。</p>
        <button class="btn primary" data-testid="pane-pick-connection" @click="emit('pickConnection', node.id)">選擇連線</button>
      </div>
      <button class="pane-close" type="button" aria-label="關閉窗格" @click="controller.closePaneLeaf(tab, node.id)">×</button>
    </div>
    <Teleport to="body">
      <template v-if="menu">
        <div class="pane-menu-backdrop" @click="closeMenu" @contextmenu.prevent="closeMenu" />
        <div class="pane-menu" role="menu" :style="{ left: `${menu.x}px`, top: `${menu.y}px` }">
          <template v-if="menuLeaf()?.kind === 'empty'">
            <button role="menuitem" @click="pickFromMenu">選擇連線…</button>
          </template>
          <template v-if="menuLeaf()?.kind === 'conn'">
            <button role="menuitem" @click="mirrorFromMenu">加入唯讀鏡像</button>
          </template>
          <div class="pane-menu-sep" />
          <button v-for="d in DIRECTIONS" :key="d.dir" role="menuitem" @click="splitFromMenu(d.dir)">{{ d.label }}</button>
          <div class="pane-menu-sep" />
          <button role="menuitem" @click="closeFromMenu">關閉窗格</button>
        </div>
      </template>
    </Teleport>
  </div>
</template>

<style scoped>
.pane-split { display: flex; flex: 1 1 0; min-width: 0; min-height: 0; position: relative; }
.pane-row { flex-direction: row; }
.pane-col { flex-direction: column; }
.pane-branch { display: flex; min-width: 0; min-height: 0; flex: 1 1 0; }
.pane-branch > :deep(.pane-cell), .pane-branch > :deep(.pane-split) { flex: 1 1 0; min-width: 0; min-height: 0; }
.pane-divider { flex: none; background: var(--border); position: relative; z-index: 2; touch-action: none; }
.pane-row > .pane-divider { width: 5px; cursor: col-resize; }
.pane-col > .pane-divider { height: 5px; cursor: row-resize; }
.pane-divider:hover, .pane-divider:focus-visible { background: var(--accent); outline: none; }
.pane-cell { position: relative; flex: 1 1 0; min-width: 0; min-height: 0; display: flex; outline: 1px solid transparent; outline-offset: -1px; }
.pane-cell.focused { outline-color: color-mix(in srgb, var(--accent) 55%, transparent); }
.pane-cell :deep(.pane) { position: relative; inset: auto; flex: 1 1 0; min-width: 0; min-height: 0; }
.pane-empty { flex: 1 1 0; min-width: 0; min-height: 0; display: flex; align-items: center; justify-content: center; background: var(--bg); }
.pane-empty-box { display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 16px; text-align: center; }
.pane-empty-box .muted { color: var(--text-dim); font-size: 12px; }
.pane-menu-backdrop { position: fixed; inset: 0; z-index: 200; }
.pane-menu { position: fixed; z-index: 201; min-width: 160px; padding: 4px; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-panel); box-shadow: 0 6px 20px #0007; }
.pane-menu button { display: block; width: 100%; text-align: left; padding: 7px 12px; border: 0; border-radius: 5px; background: transparent; color: var(--text); font: inherit; font-size: 13px; cursor: pointer; }
.pane-menu button:hover:not(:disabled) { background: var(--bg-raised); }
.pane-menu button:disabled { opacity: .45; cursor: default; }
.pane-menu-sep { height: 1px; margin: 4px 6px; background: var(--border); }
</style>
