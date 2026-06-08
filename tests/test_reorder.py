import os
import subprocess
import json
import time

def run_cmd(pinyin, select_idx=None):
    """运行测试命令并返回候选词列表"""
    # 如果 select_idx 不为 None，则模拟选择该候选词以触发记录
    cmd = f"./target/debug/qianyan-ime --test"
    input_str = f"{pinyin}\n"
    if select_idx is not None:
        input_str += " \n" * (select_idx) # 这里逻辑可能需要根据 --test 的具体交互调整
        # 由于 --test 模式目前不支持直接选择 index，我改用直接调用 record_usage 的方式或模拟空格
    
    # 简化：我们直接检查第一次搜索结果，然后模拟 'record_usage' 后的结果
    # 这里的脚本需要 qianyan-ime 支持某种方式触发 record
    process = subprocess.Popen(["./target/debug/qianyan-ime", "--test"], 
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    
    out, _ = process.communicate(input=f"{pinyin}\nexit\n")
    
    candidates = []
    found_cand = False
    for line in out.splitlines():
        if "候选词" in line:
            found_cand = True
            continue
        if found_cand and "." in line:
            parts = line.split(".")
            if len(parts) > 1:
                name = parts[1].split("(")[0].strip()
                candidates.append(name)
    return candidates

def clean_history():
    for f in ["data/learned_words.json"]:
        if os.path.exists(f):
            os.remove(f)

if __name__ == "__main__":
    print("--- 自动调频功能验证脚本 ---")
    print("⚠️  此脚本需要更新以适配新的 MRU 排序系统")
    clean_history()
    
    # 1. 初始搜索
    cands_before = run_cmd("nihao")
    if not cands_before:
        print("❌ 无法获取候选词，请确保项目已编译。")
        exit(1)
    
    print(f"初始顺序: {cands_before[:3]}")
    
    # 2. 模拟用户选择了第 2 个词 (假设是 '你会')
    target = cands_before[1]
    print(f"目标词: {target} (原排名第 2)")
    
    # 注：新系统使用 user_order (order.json)，不在 data/ 路径
    # 新系统每个 profile 的 order.json 位置：
    # ~/.config/qianyan-ime/user_data/<profile>/order.json
    # 格式: {"da": ["词1", "词2", ...]}
