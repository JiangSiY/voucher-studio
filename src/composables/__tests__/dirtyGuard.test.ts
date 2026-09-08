import { describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useDirtyGuard } from "../dirtyGuard";

/** 构造一个可变表单快照的守卫 */
function makeGuard(enabled = true) {
  const form = ref("a");
  const navigated: string[] = [];
  const guard = useDirtyGuard({
    snapshot: () => form.value,
    enabled: () => enabled,
    navigate: (p) => navigated.push(p),
  });
  return { form, guard, navigated };
}

describe("useDirtyGuard", () => {
  it("markClean 后无脏数据；改表单值后变脏", () => {
    const { form, guard } = makeGuard();
    guard.markClean();
    expect(guard.dirty.value).toBe(false);
    form.value = "b";
    expect(guard.dirty.value).toBe(true);
    guard.markClean();
    expect(guard.dirty.value).toBe(false);
  });

  it("未 markClean 时不算脏（基线为空 = 尚未加载完成）", () => {
    const { form, guard } = makeGuard();
    expect(guard.dirty.value).toBe(false);
    form.value = "changed";
    expect(guard.dirty.value).toBe(false);
  });

  it("enabled=false 时永不脏（新建模式无脏数据概念）", () => {
    const { form, guard } = makeGuard(false);
    guard.markClean();
    form.value = "b";
    expect(guard.dirty.value).toBe(false);
  });

  it("leaveTo：干净直接执行；脏数据先挂起弹确认", () => {
    const { form, guard } = makeGuard();
    guard.markClean();
    const fn = vi.fn();
    guard.leaveTo(fn);
    expect(fn).toHaveBeenCalledOnce();

    form.value = "b"; // 变脏
    guard.leaveTo(fn);
    expect(fn).toHaveBeenCalledOnce(); // 未再执行
    expect(guard.leaveConfirmVisible.value).toBe(true);
  });

  it("confirmLeave 执行挂起动作；cancelLeave 丢弃", () => {
    const { form, guard } = makeGuard();
    guard.markClean();
    form.value = "b";

    const fn = vi.fn();
    guard.leaveTo(fn);
    guard.cancelLeave();
    expect(fn).not.toHaveBeenCalled();
    expect(guard.leaveConfirmVisible.value).toBe(false);

    guard.leaveTo(fn);
    guard.confirmLeave();
    expect(fn).toHaveBeenCalledOnce();
    expect(guard.leaveConfirmVisible.value).toBe(false);
  });

  it("armNav 放行一次导航（保存并下一张场景）", () => {
    const { form, guard } = makeGuard();
    guard.markClean();
    form.value = "b";
    const fn = vi.fn();
    guard.armNav();
    guard.leaveTo(fn);
    expect(fn).toHaveBeenCalledOnce();
    expect(guard.leaveConfirmVisible.value).toBe(false);
  });

  it("beforeRouteLeave：脏数据拦截并挂起跳转，确认后 navigate 到目标", () => {
    const { form, guard, navigated } = makeGuard();
    guard.markClean();
    form.value = "b";
    expect(guard.beforeRouteLeave({ fullPath: "/settings" })).toBe(false);
    expect(navigated).toEqual([]);
    guard.confirmLeave();
    expect(navigated).toEqual(["/settings"]);
  });

  it("beforeRouteLeave：干净放行", () => {
    const { guard } = makeGuard();
    guard.markClean();
    expect(guard.beforeRouteLeave({ fullPath: "/x" })).toBe(true);
  });
});
