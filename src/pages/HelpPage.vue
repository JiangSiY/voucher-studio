<script setup lang="ts">
import { computed } from "vue";
import { useConfigStore } from "../stores/config";
import FluentIcon from "../components/FluentIcon.vue";

const configStore = useConfigStore();
const version = computed(() => configStore.appInfo?.version ?? "…");
const dataDir = computed(() => configStore.appInfo?.dataDir ?? "…");

/**
 * 快捷键清单：**只列已实装的**（真源 `docs/UI-设计规范-macOS27.md` §2.3）。
 * 规范里的 ⌘⇧Enter（复核通过并下一张）仍是规划项、代码未实现 —— 帮助页写了按下去没反应，
 * 比不写更伤（2026-10-16 界面细节审查）。⌥←/→（上一张/下一张）已于同日实装，故列出。
 */
const shortcuts: { keys: string; what: string }[] = [
  { keys: "⌘K", what: "命令面板（含备份 / 对账 / 批量识别 / 重建增强等动作）" },
  { keys: "⌘S", what: "保存当前凭证" },
  { keys: "⌘Enter", what: "保存并下一张" },
  { keys: "⌘⇧E", what: "展宽 / 收起右栏编辑器" },
  { keys: "↑ ↓", what: "在列表、摘要模板下拉里移动" },
  { keys: "Enter", what: "打开选中的凭证 / 展开当前行" },
  { keys: "⌥← ⌥→", what: "上一张 / 下一张（按当前筛选的整份结果翻，可跨页；右栏头部也有这对箭头）" },
  { keys: "Esc", what: "关闭浮层（命令面板 / 对话框 / 看图器），焦点回到原处" },
  { keys: "← →", what: "聚焦侧栏右缘手柄后微调侧栏宽度 16px；双击手柄复位 260px" },
];

/** 侧栏一列入口（与 WorkbenchLayout 的 navItems 同序同文案；帮助页不再用旧称） */
const navMap: { name: string; what: string }[] = [
  { name: "待处理", what: "该做什么就列什么（待识别 / 待核对 / 待打印 / 疑似重复 / 未填金额），全部清零后整块让位给资料" },
  { name: "全部凭证", what: "左列表 + 右编辑器，可就地改全部字段；支持拖图入库、批量改状态、批量导出" },
  { name: "重复核查", what: "同日期同金额的疑似重复，逐对判定「确认重复 / 排除」" },
  { name: "洞察", what: "时间窗驱动的月度趋势、状态分布、月度透视、导出覆盖度与存储占用" },
  { name: "打印与导出", what: "A4 多列省墨整理单、按天凭证日报，以及 Excel / CSV 台账" },
  { name: "导出历史", what: "每次导出的批次与明细，可重新打开产物" },
  { name: "设置", what: "通用 / 命名 / OCR / 增强 / 备份与同步 / 密钥 六个分区" },
  { name: "备份与恢复", what: "本地 zip 备份、WebDAV 云备份、加密；恢复写入待恢复文件后重启生效" },
  { name: "帮助", what: "本页" },
];
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>三步上手</h3></div>
    <div class="fluent-panel-body">
      <div class="help-grid">
        <div style="border: 1px solid var(--stroke); border-radius: 6px; padding: 16px">
          <div style="margin-bottom: 8px; color: var(--accent)"><FluentIcon name="image" :size="24" /></div>
          <div style="font-weight: 600; margin-bottom: 4px">1 · 收集</div>
          <div style="font-size: 12px; color: var(--text-3)">
            把票据/凭证图片拖进「全部凭证」，自动建卡入库；支持批量拖放，一图一凭证；重复图片自动去重。
          </div>
        </div>
        <div style="border: 1px solid var(--stroke); border-radius: 6px; padding: 16px">
          <div style="margin-bottom: 8px; color: var(--accent)"><FluentIcon name="edit" :size="24" /></div>
          <div style="font-weight: 600; margin-bottom: 4px">2 · 整理</div>
          <div style="font-size: 12px; color: var(--text-3)">
            补全摘要、金额、标签；一键 OCR 提取图片文字；全文搜索随时找凭证；每次修改自动存快照。
          </div>
        </div>
        <div style="border: 1px solid var(--stroke); border-radius: 6px; padding: 16px">
          <div style="margin-bottom: 8px; color: var(--accent)"><FluentIcon name="print" :size="24" /></div>
          <div style="font-weight: 600; margin-bottom: 4px">3 · 打印归档</div>
          <div style="font-size: 12px; color: var(--text-3)">
            「打印与导出」页勾选凭证，选卡片风格与列数，生成 A4 省墨整理单 PDF，用系统查看器打印归档。
          </div>
        </div>
      </div>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>界面在哪</h3></div>
    <div class="fluent-panel-body">
      <ul class="helplist">
        <li v-for="n in navMap" :key="n.name">
          <span class="hl-name">{{ n.name }}</span>
          <span class="hl-what">{{ n.what }}</span>
        </li>
      </ul>
      <p class="hl-note">
        左侧栏是一列扁平入口；底部的「收起侧边栏」把侧栏收成 56px 纯图标栏，再点同一按钮展开
        （收起后每个图标悬停都有名称提示）。侧栏右缘可拖拽调宽 220–400px，双击手柄复位。
      </p>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>快捷键</h3></div>
    <div class="fluent-panel-body">
      <ul class="helplist">
        <li v-for="s in shortcuts" :key="s.keys">
          <span class="hl-keys">{{ s.keys }}</span>
          <span class="hl-what">{{ s.what }}</span>
        </li>
      </ul>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>常见问题</h3></div>
    <div class="fluent-panel-body" style="font-size: 13px; line-height: 1.8">
      <p><strong>数据存在哪里？</strong>全部在本机：<code>{{ dataDir }}</code>（数据库 + 附件图片 + 备份）。不上传任何数据（WebDAV 备份由你显式配置；云端 OCR 需你显式切换到云端引擎）。</p>
      <p><strong>密钥安全吗？</strong>云端 OCR API Key、WebDAV 密码保存在系统钥匙串（macOS 钥匙串），不会写入文件。</p>
      <p><strong>误删了怎么办？</strong>删除前系统自动留存快照记录；数据目录损坏时用「备份与恢复」页的备份还原（恢复 = 写入待恢复文件 + 重启生效）。</p>
      <p><strong>搜索找不到凭证？</strong>全文搜索每次最多显示 100 条，结果被截断时列表上方会提示；此时把关键词写得更具体，或加上日期 / 金额筛选。图片上的文字要先完成 OCR 才可被搜到。</p>
      <p><strong>为什么没有二维码？</strong>凭证是记账原始凭证，以凭证编号为核心、打印省墨、归档可读为设计目标。</p>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-body" style="font-size: 12px; color: var(--text-3)">
      Voucher Studio v{{ version }} · Tauri 2 + Rust + Vue 3 重写版（完全无包袱重写，不迁移旧版数据）。
      本地 OCR（离线识别）已内置：RapidOCR PP-OCRv5 模型，识别全程在本机完成。
    </div>
  </div>
</template>

<style scoped>
.helplist { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
.helplist li { display: flex; align-items: baseline; gap: var(--s-3); font-size: 13px; }
.hl-name, .hl-keys { flex-shrink: 0; min-width: 96px; color: var(--label-primary); font-weight: 600; }
.hl-keys { min-width: 108px; font: 600 12.6px / 1.6 var(--font-mono); }
.hl-what { color: var(--label-secondary); }
.hl-note { margin: var(--s-3) 0 0; font-size: 12px; color: var(--text-3); line-height: 1.7; }
</style>
