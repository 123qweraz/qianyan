import os
import subprocess
import shutil

# 确保在项目根目录运行
os.chdir(os.path.dirname(os.path.abspath(__file__)) + "/..")

def run_ime_cmd(inputs):
    process = subprocess.Popen(
        ["./target/debug/qianyan-ime", "--test"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    full_input = "\n".join(inputs) + "\nexit\n"
    out, err = process.communicate(input=full_input)
    
    # 提取所有反馈动作
    lines = out.splitlines()
    actions = []
    
    for line in lines:
        if "动作反馈:" in line:
            actions.append(line.split(":")[1].strip())
            
    return actions

def test_ctrl_letter_all():
    """验证所有 Ctrl+字母 组合的按下+释放均透传为 PassThrough"""
    letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    all_ok = True

    for ch in letters:
        actions = run_ime_cmd([f"CTRL_{ch}", f"UP_CTRL_{ch}"])
        if len(actions) < 2 or any("PassThrough" not in a for a in actions[:2]):
            print(f"  ❌ Ctrl+{ch} 失败: {actions}")
            all_ok = False
    if all_ok:
        print("✅ [测试 4] 所有 Ctrl+字母 按下+释放均正确透传")
    return all_ok

def test_ctrl_d_passthrough():
    """具体验证 Ctrl+D 透传 (Bug 回归验证)"""
    actions = run_ime_cmd(["CTRL_D", "UP_CTRL_D"])
    ok = len(actions) >= 2 and all("PassThrough" in a for a in actions[:2])
    print(f"{'✅' if ok else '❌'} [测试 5] Ctrl+D 透传: {actions}")
    return ok

def test_ctrl_letter_with_buffer():
    """Buffer 不为空时 Ctrl+字母 仍透传"""
    all_ok = True
    for ch in "DWEGT":
        actions = run_ime_cmd(["n", "i", f"CTRL_{ch}", f"UP_CTRL_{ch}"])
        # 第1-2个是 Emit(n), Emit(i), 第3-4个是 Ctrl 动作
        ctrl_actions = actions[2:]
        if len(ctrl_actions) < 2 or any("PassThrough" not in a for a in ctrl_actions):
            print(f"  ❌ Ctrl+{ch} (有buffer) 失败: {actions}")
            all_ok = False
    if all_ok:
        print("✅ [测试 6] Buffer 占用时 Ctrl+字母 仍正确透传")
    return all_ok

def test_ctrl_alt_letter_passthrough():
    """验证 Ctrl+Alt+字母 全部透传"""
    all_ok = True
    for ch in "ABCDXYZ":
        actions = run_ime_cmd([f"CTRL_ALT_{ch}", f"UP_CTRL_ALT_{ch}"])
        if len(actions) < 2 or any("PassThrough" not in a for a in actions[:2]):
            print(f"  ❌ Ctrl+Alt+{ch} 失败: {actions}")
            all_ok = False
    if all_ok:
        print("✅ [测试 7] Ctrl+Alt+字母 正确透传")
    return all_ok

def test_letter_input_still_works():
    """验证普通字母输入不受影响"""
    actions = run_ime_cmd(["n", "i"])
    # 应该得到 Emit(n) 和 Emit(i) 或类似的处理动作
    ok = len(actions) >= 2
    print(f"{'✅' if ok else '❌'} [测试 8] 普通字母输入不受影响")
    return ok


if __name__ == "__main__":
    import sys
    print("--- 系统快捷键回归测试 ---")
    
    # 1. 测试 Ctrl + C 的完整生命周期
    print("\n[测试 1] 验证 Ctrl + C 的按下与释放...")
    actions1 = run_ime_cmd(["CTRL_C", "UP_CTRL_C"])
    print(f"动作序列: {actions1}")
    
    if len(actions1) >= 2 and all("PassThrough" in a for a in actions1):
        print("✅ [成功] Ctrl + C 的所有事件均已正确透传")
    else:
        print("❌ [失败] 部分事件被拦截，可能导致按键粘连")

    # 2. 测试在 Buffer 不为空时使用快捷键
    print("\n[测试 2] 验证 Buffer 不为空时快捷键的透传...")
    actions2 = run_ime_cmd(["w", "o", "CTRL_C", "UP_CTRL_C"])
    shortcut_actions = actions2[2:] if len(actions2) > 2 else []
    print(f"快捷键反馈: {shortcut_actions}")
    
    if shortcut_actions and all("PassThrough" in a for a in shortcut_actions):
        print("✅ [成功] Buffer 占用时，快捷键释放事件依然正确透传")
    else:
        print("❌ [失败] Buffer 占用导致释放事件被拦截")

    results = []
    results.append(("Ctrl+字母全部透传", test_ctrl_letter_all()))
    results.append(("Ctrl+D 透传回归", test_ctrl_d_passthrough()))
    results.append(("Buffer 占用 Ctrl+字母", test_ctrl_letter_with_buffer()))
    results.append(("Ctrl+Alt+字母透传", test_ctrl_alt_letter_passthrough()))
    results.append(("普通字母输入正常", test_letter_input_still_works()))

    print("\n--- 测试汇总 ---")
    all_pass = True
    for name, ok in results:
        status = "✅" if ok else "❌"
        print(f"  {status} {name}")
        if not ok:
            all_pass = False

    if not all_pass:
        sys.exit(1)
