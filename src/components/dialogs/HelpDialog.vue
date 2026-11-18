<script setup lang="ts">
/**
 * 帮助弹层（v0.27）：只列已实装快捷键与当前四页签。
 * 旧 `HelpPage` 在 B4 迁移完成后删除；本组件是唯一帮助入口（头像菜单 / `?modal=help`）。
 */
import Dialog from "../ui/Dialog.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const { close } = useModalRoute();

const shortcuts: { keys: string; what: string }[] = [
  { keys: "⌘K", what: "命令面板（页面跳转 / 新建条目 / 记录搜索）" },
  { keys: "⌘S", what: "保存当前展开的条目" },
  { keys: "⌘Enter", what: "保存并展开下一条" },
  { keys: "⌘⇧E", what: "标记当前条目已导出" },
  { keys: "⌘⇧Enter", what: "复核通过并展开下一条" },
  { keys: "⌥← ⌥→", what: "上一条 / 下一条" },
  { keys: "Esc", what: "关闭弹层或命令面板" },
];
</script>

<template>
  <Dialog :open="true" title="帮助" @close="close">
    <section class="help-section">
      <h3 class="help-title">四个页签</h3>
      <ul class="help-list">
        <li><b>工作台</b>：关键数字、趋势/排行/占比、待办下钻。</li>
        <li><b>数据</b>：条目库；点行展开编辑，新建/导入/打印导出都从这里进。</li>
        <li><b>对账</b>：选台账 CSV，比对差异并导出。</li>
        <li><b>会计投影</b>：期间 → 计划 → 分录 → 映射，只读投影可重算。</li>
      </ul>
    </section>

    <section class="help-section">
      <h3 class="help-title">条目与凭证</h3>
      <p class="help-text">
        一条记录 =
        一个数据条目；凭证是该条目的唯一附件，新建时必填。无凭证的条目不能入账、导出或归档；历史无凭证条目可先补图再继续。
      </p>
      <p class="help-text">列表每页最多 100 条；搜索与命令面板均走同一个后端索引。</p>
    </section>

    <section class="help-section">
      <h3 class="help-title">快捷键</h3>
      <dl class="help-keys">
        <template v-for="s in shortcuts" :key="s.keys">
          <dt>
            <kbd>{{ s.keys }}</kbd>
          </dt>
          <dd>{{ s.what }}</dd>
        </template>
      </dl>
    </section>

    <section class="help-section">
      <h3 class="help-title">平台</h3>
      <p class="help-text">
        macOS 桌面版用「macOS 钥匙串」保存密钥；手机用浏览器/PWA 打开本机服务，数据不出本机。
      </p>
    </section>
  </Dialog>
</template>

<style scoped>
.help-section {
  padding: var(--s-3) 0;
  border-bottom: 1px solid var(--separator);
}

.help-section:last-child {
  border-bottom: none;
}

.help-title {
  margin: 0 0 var(--s-2);
  font: var(--type-title);
}

.help-text,
.help-list {
  margin: 0;
  color: var(--text-2);
  font: var(--type-body);
}

.help-list {
  padding-left: var(--s-5);
}

.help-keys {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--s-2) var(--s-4);
  margin: 0;
}

.help-keys dt {
  margin: 0;
}

.help-keys dd {
  margin: 0;
  color: var(--text-2);
  font: var(--type-body);
}

kbd {
  display: inline-block;
  padding: 0 var(--s-2);
  border: 1px solid var(--separator);
  border-radius: var(--radius-small);
  background: var(--surface-2);
  font: var(--type-caption);
}
</style>
