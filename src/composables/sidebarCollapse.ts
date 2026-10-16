/**
 * 侧栏收起状态的 UI 应用（与 `useSidebarResize.ts` 的 `--sidebar-w` 同一思路）。
 *
 * 收起**不写元素内联样式、也不加组件类名**，而是写根属性 `data-sidebar-collapsed`：
 *  - 选择器 `:root[data-sidebar-collapsed="true"] .sidebar` 特异性 0,3,0，高于媒体查询里的
 *    `.sidebar`（0,1,0），故窄窗档位（≤900 的 `min(--sidebar-w, 35vw)`、≤760 的 56px）
 *    不会被顶掉——内联样式会（教训见 `useSidebarResize.ts` 头注释）；
 *  - 与主题（`data-theme`）走同一条 JS→CSS 通道，浏览器预览模式（config 为 null）也能立刻生效。
 *
 * 状态本身（用户偏好，跨启动保持）由 `stores/config.ts` 持有并落盘
 * `appearance.sidebarCollapsed`；本模块只负责把它映射到 DOM。
 */

/** 写入根属性：`"true"` = 收起、`"false"` = 展开（属性缺失时 CSS 视作展开，只匹配 "true"） */
export function applySidebarCollapsed(collapsed: boolean): void {
  document.documentElement.dataset.sidebarCollapsed = collapsed ? "true" : "false";
}
