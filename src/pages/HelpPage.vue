<script setup lang="ts">
import { computed } from "vue";
import { useConfigStore } from "../stores/config";
import FluentIcon from "../components/FluentIcon.vue";

const configStore = useConfigStore();
const version = computed(() => configStore.appInfo?.version ?? "…");
const dataDir = computed(() => configStore.appInfo?.dataDir ?? "…");
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
            把票据/凭证图片拖进「凭证列表」，自动建卡入库；支持批量拖放，一图一凭证；重复图片自动去重。
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
            「打印中心」勾选凭证，选卡片风格与列数，生成 A4 省墨整理单 PDF，用系统查看器打印归档。
          </div>
        </div>
      </div>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header"><h3>常见问题</h3></div>
    <div class="fluent-panel-body" style="font-size: 13px; line-height: 1.8">
      <p><strong>数据存在哪里？</strong>全部在本机：<code>{{ dataDir }}</code>（数据库 + 附件图片 + 备份）。不上传任何数据（WebDAV 备份由你显式配置）。</p>
      <p><strong>密钥安全吗？</strong>OCR API Key、WebDAV 密码保存在操作系统凭据库（Windows 凭据管理器），不会写入文件。</p>
      <p><strong>误删了怎么办？</strong>删除前系统自动留存快照记录；数据目录损坏时用「备份与恢复」页的备份还原（恢复 = 就绪 + 重启生效）。</p>
      <p><strong>为什么没有二维码？</strong>凭证是记账原始凭证，以凭证编号为核心、打印省墨、归档可读为设计目标。</p>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-body" style="font-size: 12px; color: var(--text-3)">
      Voucher Studio v{{ version }} · Tauri 2 + Rust + Vue 3 重写版（完全无包袱重写，不迁移旧版数据）。
      本地 OCR（离线识别）已内置：RapidOCR PP-OCRv4 模型，G1 Spike B 实测字符准确率 99.9%。
    </div>
  </div>
</template>
