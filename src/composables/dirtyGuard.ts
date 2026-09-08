/**
 * 脏数据守卫（v0.9 动线提取）：表单基线比对 + 未保存离开确认。
 * 与路由解耦——navigate 由调用方注入（页面传 router.push，测试传 spy）。
 *
 * 用法：
 *   const guard = useDirtyGuard({
 *     snapshot: () => JSON.stringify([...表单字段]),
 *     enabled: () => editId != null,
 *     navigate: (p) => router.push(p),
 *   });
 *   // 加载/保存成功后 guard.markClean()；离开前 guard.leaveTo(fn)
 *   // 路由守卫：onBeforeRouteLeave((to) => guard.beforeRouteLeave(to))
 */
import { computed, ref, type ComputedRef, type Ref } from "vue";

export interface DirtyGuardOptions {
  /** 表单当前值序列化（任何字段变化都会改变返回值） */
  snapshot: () => string;
  /** 是否启用守卫（编辑模式才启用；新建模式无脏数据概念） */
  enabled: () => boolean;
  /** 路由跳转（确认离开后执行；组件注入 router.push） */
  navigate: (path: string) => void;
}

export interface DirtyGuard {
  /** 有未保存修改（启用且当前值偏离基线） */
  dirty: ComputedRef<boolean>;
  /** 确认对话框可见性（绑定 ConfirmDialog :visible） */
  leaveConfirmVisible: Ref<boolean>;
  /** 确认对话框文案（绑定 ConfirmDialog :body） */
  readonly leaveConfirmText: string;
  /** 记录当前值为新基线（详情加载 / 保存成功后调用） */
  markClean: () => void;
  /** 已确认离开一次：下一次导航直接放行（保存并下一张等场景） */
  armNav: () => void;
  /** 离页口：脏数据先弹确认，干净/armed 直接执行 */
  leaveTo: (fn: () => void) => void;
  /** 用户点「离开」：执行挂起动作 */
  confirmLeave: () => void;
  /** 用户点「取消」：丢弃挂起动作 */
  cancelLeave: () => void;
  /** vue-router 守卫：返回 true 放行 / false 拦截（拦截时弹确认） */
  beforeRouteLeave: (to: { fullPath: string }) => boolean;
}

export function useDirtyGuard(opts: DirtyGuardOptions): DirtyGuard {
  const baseline = ref("");
  const dirty = computed(
    () => opts.enabled() && baseline.value !== "" && opts.snapshot() !== baseline.value,
  );

  const leaveConfirmVisible = ref(false);
  const leaveConfirmText =
    "当前凭证有未保存的修改，离开将丢失这些改动。确定离开吗？";
  let pendingNavFn: (() => void) | null = null;
  let navArmed = false;

  function markClean() {
    baseline.value = opts.snapshot();
    navArmed = false;
  }

  function armNav() {
    navArmed = true;
  }

  function leaveTo(fn: () => void) {
    if (!dirty.value || navArmed) {
      fn();
      return;
    }
    pendingNavFn = fn;
    leaveConfirmVisible.value = true;
  }

  function confirmLeave() {
    leaveConfirmVisible.value = false;
    navArmed = true;
    const fn = pendingNavFn;
    pendingNavFn = null;
    fn?.();
  }

  function cancelLeave() {
    leaveConfirmVisible.value = false;
    pendingNavFn = null;
  }

  function beforeRouteLeave(to: { fullPath: string }): boolean {
    if (!dirty.value || navArmed) {
      navArmed = false;
      return true;
    }
    pendingNavFn = () => {
      navArmed = true;
      opts.navigate(to.fullPath);
    };
    leaveConfirmVisible.value = true;
    return false;
  }

  return {
    dirty,
    leaveConfirmVisible,
    leaveConfirmText,
    markClean,
    armNav,
    leaveTo,
    confirmLeave,
    cancelLeave,
    beforeRouteLeave,
  };
}
