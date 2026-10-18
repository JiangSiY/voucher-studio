<script setup lang="ts">
/**
 * v0.12 图片手动裁剪 / 旋转对话框（手写组件，无第三方库）。
 *
 * 语义：以原图为基准 → 旋转（±90° 整数倍 + ±15° 微调）→ 框选裁剪 →
 * 覆盖写增强版（后端 crop_attachment）。旋转约定与后端一致：正角度 = 顺时针。
 *
 * 坐标系：裁剪框相对「旋转后图像的显示包围盒」；保存时按显示比例换算回
 * 旋转后图像的像素坐标（后端先旋转再裁剪，同一坐标语义）。
 */
import { computed, onBeforeUnmount, ref, watch } from "vue";
import FluentIcon from "./FluentIcon.vue";
import { useModal } from "../composables/useModal";

const props = defineProps<{
  visible: boolean;
  attachmentId: number | null;
  /** 原图 URL（裁剪基准；不用增强版——手动裁剪是对自动增强的替代而非叠加） */
  src: string;
  fileName: string;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", rect: { x: number; y: number; w: number; h: number }, rotationDeg: number): void;
}>();

/** 焦点契约（规范 §4）：打开聚焦首个可操作元素、Tab 循环留在浮层内、Esc 关闭并归还焦点 */
const { panelEl } = useModal(
  () => props.visible,
  () => emit("close"),
);

// ---------- 图像几何（原图像素 → 旋转包围盒 → 显示尺寸） ----------

const naturalW = ref(0);
const naturalH = ref(0);
const STAGE_MAX = 520; // 舞台最大边（显示）

/** 旋转状态：90° 整数倍 + 微调（合计为总角度；正 = 顺时针，与后端一致） */
const quarterTurns = ref(0);
const fineDeg = ref(0);
const totalDeg = computed(() => quarterTurns.value * 90 + fineDeg.value);

/** 旋转后图像尺寸（像素，与后端 bounding box 公式一致） */
const rotatedSize = computed(() => {
  const rad = (totalDeg.value * Math.PI) / 180;
  const c = Math.abs(Math.cos(rad));
  const s = Math.abs(Math.sin(rad));
  return {
    w: Math.ceil(naturalW.value * c + naturalH.value * s),
    h: Math.ceil(naturalW.value * s + naturalH.value * c),
  };
});

/** 显示缩放：旋转包围盒适配舞台 */
const displayScale = computed(() => {
  const { w, h } = rotatedSize.value;
  if (!w || !h) return 1;
  return Math.min(STAGE_MAX / w, STAGE_MAX / h, 1);
});

const dispRot = computed(() => ({
  w: Math.max(1, Math.round(rotatedSize.value.w * displayScale.value)),
  h: Math.max(1, Math.round(rotatedSize.value.h * displayScale.value)),
}));

/** 原图显示尺寸（未旋转的 layout 尺寸；img 用 CSS rotate 旋转） */
const dispNatural = computed(() => {
  if (!naturalW.value || !naturalH.value) return { w: 0, h: 0 };
  const s = displayScale.value;
  return {
    w: Math.round(naturalW.value * s),
    h: Math.round(naturalH.value * s),
  };
});

// ---------- 裁剪框（显示坐标，相对旋转包围盒） ----------

const box = ref({ x: 0, y: 0, w: 0, h: 0 });
type DragMode = "move" | "nw" | "ne" | "sw" | "se" | null;
let drag: DragMode = null;
let dragStart = { mx: 0, my: 0, box: { x: 0, y: 0, w: 0, h: 0 } };

const MIN_BOX = 24;

function resetBox() {
  const { w, h } = dispRot.value;
  if (!w || !h) return;
  box.value = {
    x: Math.round(w * 0.05),
    y: Math.round(h * 0.05),
    w: Math.round(w * 0.9),
    h: Math.round(h * 0.9),
  };
}

function clampBox() {
  const { w: rw, h: rh } = dispRot.value;
  const b = box.value;
  b.w = Math.max(MIN_BOX, Math.min(b.w, rw));
  b.h = Math.max(MIN_BOX, Math.min(b.h, rh));
  b.x = Math.max(0, Math.min(b.x, rw - b.w));
  b.y = Math.max(0, Math.min(b.y, rh - b.h));
}

function onPointerDown(e: MouseEvent, mode: DragMode) {
  // macOS 的 Ctrl+左键是次要点击（等价右键），不得拖走裁剪框（2026-09-23 审计 B17）
  if (e.button !== 0 || e.ctrlKey) return;
  e.preventDefault();
  e.stopPropagation();
  drag = mode;
  dragStart = { mx: e.clientX, my: e.clientY, box: { ...box.value } };
  window.addEventListener("mousemove", onPointerMove);
  window.addEventListener("mouseup", onPointerUp);
}

function onPointerMove(e: MouseEvent) {
  if (!drag) return;
  const dx = e.clientX - dragStart.mx;
  const dy = e.clientY - dragStart.my;
  const b0 = dragStart.box;
  if (drag === "move") {
    box.value = { ...box.value, x: b0.x + dx, y: b0.y + dy };
  } else {
    // 角柄：固定对角，重新计算位置与尺寸
    let x1 = b0.x;
    let y1 = b0.y;
    let x2 = b0.x + b0.w;
    let y2 = b0.y + b0.h;
    if (drag === "nw") {
      x1 += dx;
      y1 += dy;
    } else if (drag === "ne") {
      x2 += dx;
      y1 += dy;
    } else if (drag === "sw") {
      x1 += dx;
      y2 += dy;
    } else if (drag === "se") {
      x2 += dx;
      y2 += dy;
    }
    box.value = {
      x: Math.min(x1, x2),
      y: Math.min(y1, y2),
      w: Math.abs(x2 - x1),
      h: Math.abs(y2 - y1),
    };
  }
  clampBox();
}

function onPointerUp() {
  drag = null;
  window.removeEventListener("mousemove", onPointerMove);
  window.removeEventListener("mouseup", onPointerUp);
}

onBeforeUnmount(() => {
  window.removeEventListener("mousemove", onPointerMove);
  window.removeEventListener("mouseup", onPointerUp);
});

// ---------- 旋转操作（换向时重置裁剪框到居中 90%） ----------

function rotateQuarter(dir: 1 | -1) {
  quarterTurns.value += dir;
  // 归一到 0–3（与后端 rem_euclid 无关——这里只是显示圈数）
  quarterTurns.value = ((quarterTurns.value % 4) + 4) % 4;
  resetBox();
}

watch(fineDeg, () => {
  // 微调改变包围盒 → clamp 现有框（保持用户选择尽量不变）
  clampBox();
});

// ---------- 加载图像尺寸 ----------

watch(
  () => [props.visible, props.src] as const,
  ([visible, src]) => {
    if (!visible || !src) return;
    const img = new Image();
    img.onload = () => {
      naturalW.value = img.naturalWidth;
      naturalH.value = img.naturalHeight;
      resetBox();
    };
    img.src = src;
  },
  { immediate: true },
);

// ---------- 保存：显示坐标 → 旋转后图像像素坐标 ----------

const saving = ref(false);

function save() {
  const s = displayScale.value;
  if (!s || box.value.w < MIN_BOX || box.value.h < MIN_BOX) return;
  const rect = {
    x: Math.max(0, Math.round(box.value.x / s)),
    y: Math.max(0, Math.round(box.value.y / s)),
    w: Math.max(1, Math.round(box.value.w / s)),
    h: Math.max(1, Math.round(box.value.h / s)),
  };
  saving.value = true;
  emit("save", rect, totalDeg.value);
}

function done() {
  saving.value = false;
}

function resetAll() {
  quarterTurns.value = 0;
  fineDeg.value = 0;
  resetBox();
}

defineExpose({ done });

const HANDLES: { key: Exclude<DragMode, null>; cls: string }[] = [
  { key: "nw", cls: "h-nw" },
  { key: "ne", cls: "h-ne" },
  { key: "sw", cls: "h-sw" },
  { key: "se", cls: "h-se" },
];
</script>

<template>
  <Teleport to="body">
    <div v-if="visible" class="crop-mask" @click.self="emit('close')">
      <div ref="panelEl" class="crop-dialog" role="dialog" aria-modal="true" aria-label="调整图片" tabindex="-1">
        <h3>调整图片 <span class="crop-sub">（裁剪 / 旋转 → 覆盖增强版；原图不受影响）</span></h3>

        <!-- 舞台：旋转后包围盒 + 裁剪框 -->
        <div class="crop-stage" :style="{ width: dispRot.w + 'px', height: dispRot.h + 'px' }"
          @mousedown="onPointerDown($event, 'move')">
          <img class="crop-img" :src="src" :alt="fileName" draggable="false"
            :style="{
              width: dispNatural.w + 'px',
              height: dispNatural.h + 'px',
              transform: `rotate(${totalDeg}deg)`,
            }" />
          <div class="crop-box" :style="{
            left: box.x + 'px', top: box.y + 'px',
            width: box.w + 'px', height: box.h + 'px',
          }" @mousedown="onPointerDown($event, 'move')">
            <div v-for="h in HANDLES" :key="h.key" :class="['crop-handle', h.cls]"
              @mousedown="onPointerDown($event, h.key)"></div>
          </div>
        </div>

        <!-- 操作条 -->
        <div class="crop-ops">
          <div class="crop-rot">
            <button class="btn btn-sm" title="左转 90°" @click="rotateQuarter(-1)">
              <FluentIcon name="rotate" :size="14" style="transform: scaleX(-1)" />左转
            </button>
            <button class="btn btn-sm" title="右转 90°" @click="rotateQuarter(1)">
              <FluentIcon name="rotate" :size="14" />右转
            </button>
            <label class="fine-slider">
              微调
              <input type="range" min="-15" max="15" step="1" v-model.number="fineDeg" />
              <span class="fine-val">{{ fineDeg }}°</span>
            </label>
            <button class="btn btn-sm" @click="resetAll">重置</button>
          </div>
          <div class="crop-save">
            <span class="crop-hint">拖动裁剪框与四角手柄调整范围</span>
            <button class="btn" :disabled="saving" @click="emit('close')">取消</button>
            <button class="btn btn-primary" :disabled="saving" @click="save">
              {{ saving ? "保存中…" : "保存调整" }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.crop-mask {
  position: fixed;
  inset: 0;
  z-index: 200;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
}

.crop-dialog {
  background: var(--surface);
  border-radius: 10px;
  padding: 16px 20px 14px;
  max-width: 92vw;
  max-height: 92vh;
  overflow: auto;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.28);
}

.crop-dialog h3 {
  margin: 0 0 10px;
  font-size: var(--fs-body);
}

.crop-sub {
  font-size: var(--fs-caption);
  color: var(--text-3);
  font-weight: 400;
}

.crop-stage {
  position: relative;
  margin: 0 auto;
  user-select: none;
  background:
    repeating-conic-gradient(var(--bg-window) 0% 25%, var(--bg-control) 0% 50%) 0 0 / 16px 16px;
  border: 1px solid var(--stroke);
  cursor: move;
  overflow: hidden;
}

.crop-img {
  position: absolute;
  left: 50%;
  top: 50%;
  transform-origin: center;
  pointer-events: none;
  /* 让 translate 居中与 rotate 叠加：先居中再旋转 */
  translate: -50% -50%;
}

.crop-box {
  position: absolute;
  border: 1.5px solid var(--accent);
  box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.38);
  cursor: move;
}

.crop-handle {
  position: absolute;
  width: 12px;
  height: 12px;
  background: var(--accent);
  border: 2px solid var(--label-on-accent);
  border-radius: var(--radius-small);
}

.h-nw { left: -7px; top: -7px; cursor: nwse-resize; }
.h-ne { right: -7px; top: -7px; cursor: nesw-resize; }
.h-sw { left: -7px; bottom: -7px; cursor: nesw-resize; }
.h-se { right: -7px; bottom: -7px; cursor: nwse-resize; }

.crop-ops {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  margin-top: 12px;
  flex-wrap: wrap;
}

.crop-rot {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
}

.fine-slider {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-caption);
  color: var(--text-2);
}

.fine-slider input[type="range"] {
  width: 120px;
}

.fine-val {
  min-width: 28px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

.crop-save {
  display: flex;
  gap: 8px;
  align-items: center;
}

.crop-hint {
  font-size: var(--fs-caption);
  color: var(--text-4);
  margin-right: 4px;
}
</style>
