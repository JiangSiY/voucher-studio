<script setup lang="ts">
/**
 * 图片查看器（手写组件，无第三方依赖）：
 * 滚轮缩放（鼠标锚点）/ 拖拽平移 / 双击切换适应与 100% / 90° 旋转 / 键盘快捷键。
 * 兜底：「系统打开」用系统默认看图程序打开原图（图过大卡顿时逃生）。
 */
import { computed, onUnmounted, ref, watch } from "vue";
import { api } from "../api/client";
import FluentIcon from "./FluentIcon.vue";
import { toastError } from "../stores/toast";

const props = defineProps<{
  visible: boolean;
  src: string;
  fileName: string;
  /** 附件 id；「系统打开」需要绝对路径（null 时隐藏该按钮） */
  attachmentId: number | null;
}>();
const emit = defineEmits<{ (e: "close"): void }>();

const MIN_SCALE = 0.05;
const MAX_SCALE = 12;

const scale = ref(1);
const x = ref(0);
const y = ref(0);
const rotation = ref(0);
const fitScale = ref(1);
const dragging = ref(false);
const imgEl = ref<HTMLImageElement | null>(null);

const transform = computed(
  () => `translate(${x.value}px, ${y.value}px) scale(${scale.value}) rotate(${rotation.value}deg)`,
);
const zoomLabel = computed(() => `${Math.round(scale.value * 100)}%`);

watch(
  () => props.visible,
  (v) => {
    if (v) {
      resetView();
      // D-7：查看器打开即落一条 viewer 命中（后端校验：无增强版的附件不计入）
      // 静默失败——埋点失败绝不能影响看图
      if (props.attachmentId != null) {
        api.recordViewerHit(props.attachmentId).catch(() => {});
      }
    }
    document.body.style.overflow = v ? "hidden" : "";
  },
);

// 键盘：Esc 关闭、0 重置、+/- 缩放
function onKeydown(e: KeyboardEvent) {
  if (!props.visible) return;
  if (e.key === "Escape") emit("close");
  else if (e.key === "0") resetView();
  else if (e.key === "+" || e.key === "=") zoomCenter(1.25);
  else if (e.key === "-") zoomCenter(0.8);
}
window.addEventListener("keydown", onKeydown);
window.addEventListener("resize", onResize);
onUnmounted(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("resize", onResize);
  document.body.style.overflow = "";
});

/** 当前窗口下"适应窗口"的比例（图片按自然尺寸渲染，transform scale 控制显示大小） */
function computeFit(): number {
  const el = imgEl.value;
  if (!el || !el.naturalWidth) return 0;
  const cw = window.innerWidth - 64;
  const ch = window.innerHeight - 140;
  return Math.min(cw / el.naturalWidth, ch / el.naturalHeight, 1);
}

function onImgLoad() {
  const el = imgEl.value;
  if (!el) return;
  const fit = computeFit();
  const wasFitted = Math.abs(scale.value - fitScale.value) < 0.001;
  fitScale.value = fit;
  if (scale.value === 1 && x.value === 0 && y.value === 0) {
    scale.value = fit;
  } else if (wasFitted) {
    // 原先处于"适应"状态 → 跟随新的适应比例
    scale.value = fit;
  }
}

/** 窗口尺寸变化时重算适应比例：原先只在图片 load 时算一次，
 *  缩放窗口后「适应」按钮/双击/按 0 会回到过时的比例 */
function onResize() {
  if (!imgEl.value) return;
  const fit = computeFit();
  const wasFitted = Math.abs(scale.value - fitScale.value) < 0.001;
  fitScale.value = fit;
  if (wasFitted) scale.value = fit;
}

function resetView() {
  x.value = 0;
  y.value = 0;
  rotation.value = 0;
  scale.value = fitScale.value || 1;
}

/** 双击：适应窗口 ↔ 100% 切换 */
function toggleFit() {
  if (Math.abs(scale.value - fitScale.value) < 0.01) {
    scale.value = 1;
    x.value = 0;
    y.value = 0;
  } else {
    scale.value = fitScale.value;
    x.value = 0;
    y.value = 0;
  }
}

function clampScale(s: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));
}

/** 以窗口中心为锚缩放 */
function zoomCenter(factor: number) {
  const ns = clampScale(scale.value * factor);
  x.value *= ns / scale.value;
  y.value *= ns / scale.value;
  scale.value = ns;
}

/** 滚轮缩放：未旋转时以鼠标位置为锚（内容点保持在鼠标下），旋转态退化为中心锚 */
function onWheel(e: WheelEvent) {
  const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
  const ns = clampScale(scale.value * factor);
  if (ns === scale.value) return;
  const cx = window.innerWidth / 2;
  const cy = window.innerHeight / 2;
  const rot = ((rotation.value % 360) + 360) % 360;
  if (rot === 0) {
    const ratio = ns / scale.value;
    x.value = e.clientX - cx - (e.clientX - cx - x.value) * ratio;
    y.value = e.clientY - cy - (e.clientY - cy - y.value) * ratio;
  } else {
    x.value *= ns / scale.value;
    y.value *= ns / scale.value;
  }
  scale.value = ns;
}

function rotate() {
  rotation.value = (rotation.value + 90) % 360;
}

// 拖拽平移
let dragStartX = 0;
let dragStartY = 0;
let dragBaseX = 0;
let dragBaseY = 0;

function onMouseDown(e: MouseEvent) {
  // macOS 的 Ctrl+左键是次要点击（等价右键），不得拖动画布（2026-09-23 审计 B17）
  if (e.button !== 0 || e.ctrlKey) return;
  dragging.value = true;
  dragStartX = e.clientX;
  dragStartY = e.clientY;
  dragBaseX = x.value;
  dragBaseY = y.value;
}
function onMouseMove(e: MouseEvent) {
  if (!dragging.value) return;
  x.value = dragBaseX + (e.clientX - dragStartX);
  y.value = dragBaseY + (e.clientY - dragStartY);
}
function endDrag() {
  dragging.value = false;
}

/** 兜底：系统默认程序打开原图 */
async function openExternal() {
  if (props.attachmentId == null) return;
  try {
    const abs = await api.attachmentAbsPath(props.attachmentId);
    await api.openPath(abs);
  } catch (e) {
    toastError(e);
  }
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="visible"
      class="viewer-mask"
      :class="{ grabbing: dragging }"
      @wheel.prevent="onWheel"
      @mousedown="onMouseDown"
      @mousemove="onMouseMove"
      @mouseup="endDrag"
      @mouseleave="endDrag"
      @dblclick="toggleFit"
    >
      <img
        ref="imgEl"
        :src="src"
        :alt="fileName"
        class="viewer-img"
        :style="{ transform }"
        draggable="false"
        @load="onImgLoad"
      />

      <div class="viewer-toolbar" @mousedown.stop @wheel.stop @dblclick.stop>
        <button class="vbtn" title="放大（滚轮/+=）" @click="zoomCenter(1.25)"><FluentIcon name="zoomIn" :size="14" /></button>
        <span class="viewer-zoom">{{ zoomLabel }}</span>
        <button class="vbtn" title="缩小（滚轮/-）" @click="zoomCenter(0.8)"><FluentIcon name="zoomOut" :size="14" /></button>
        <button class="vbtn" title="旋转 90°" @click="rotate"><FluentIcon name="rotate" :size="14" /></button>
        <button class="vbtn" title="重置（0）" @click="resetView">适配</button>
        <button v-if="attachmentId != null" class="vbtn" title="用系统默认看图程序打开原图"
          @click="openExternal">系统打开</button>
        <button class="vbtn vbtn-close" title="关闭（Esc）" @click="emit('close')"><FluentIcon name="close" :size="14" /></button>
      </div>

      <div class="viewer-info" @mousedown.stop>{{ fileName }} · {{ zoomLabel }}</div>
    </div>
  </Teleport>
</template>

<style scoped>
.viewer-mask {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(0, 0, 0, 0.82);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: grab;
  user-select: none;
}
.viewer-mask.grabbing {
  cursor: grabbing;
}
.viewer-img {
  max-width: none;
  max-height: none;
  will-change: transform;
  pointer-events: none; /* 拖拽事件由遮罩统一处理 */
}
.viewer-toolbar {
  position: absolute;
  top: 16px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-radius: 8px;
  background: rgba(30, 30, 30, 0.85);
  border: 1px solid rgba(255, 255, 255, 0.14);
}
.viewer-zoom {
  min-width: 44px;
  text-align: center;
  font-size: 12px;
  color: #d5d5d5;
  font-variant-numeric: tabular-nums;
}
.vbtn {
  border: none;
  background: transparent;
  color: #e8e8e8;
  font-size: 13px;
  padding: 4px 10px;
  border-radius: 6px;
  cursor: pointer;
}
.vbtn:hover {
  background: rgba(255, 255, 255, 0.12);
}
.vbtn-close:hover {
  background: rgba(232, 70, 58, 0.7);
}
.viewer-info {
  position: absolute;
  bottom: 14px;
  left: 50%;
  transform: translateX(-50%);
  max-width: 80vw;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: #b9b9b9;
  background: rgba(30, 30, 30, 0.75);
  padding: 4px 12px;
  border-radius: 999px;
}
</style>
