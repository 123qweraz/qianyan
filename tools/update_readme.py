import re

file_path = 'README.md'
content = open(file_path, 'r', encoding='utf-8').read()

# 1. Update English Aux section
en_aux_new = """### 1. 英文辅助码：义向筛选
这是 rust-ime 的核心特色。当拼音重码较多时，无需翻页，只需输入 **Shift + 大写字母** 即可根据“英文释义”精准定位。

*   **逻辑**：系统扫描候选词的英文注释，匹配首字母。
*   **示例**：
    *   输入 `li` ➔ 候选：里、离、礼、理...
    *   输入 `liC` (Shift+C) ➔ **礼** (Ceremony) 瞬间排到第一。
    *   输入 `liL` (Shift+L) ➔ **理** (Logic) 瞬间排到第一。
*   **并存**：可与笔画辅助码同时使用（见下文）。
"""

content = re.sub(r'### 1\. 英文辅助码：.*?### 2\.', en_aux_new + '
### 2.', content, flags=re.DOTALL)

# 2. Update Stroke Aux section
stroke_aux_new = """### 2. 笔画辅助码 (SBSRF 4码系统)：形向筛选
如果您更习惯根据汉字的形状来筛选，可以使用基于 **SBSRF (笔画森林)** 逻辑的 4 码辅助系统。

*   **输入方式**：在拼音后输入 **分号 `;`** 键，随后输入笔画映射码（小写字母）。
*   **布局逻辑 (5x5 矩阵)**：将 5 种基本笔画映射到 QWERTY 区域：
    *   **行**：代表该对笔画中的 **第 1 笔**。
    *   **列**：代表该对笔画中的 **第 2 笔**。
    *   **笔画索引**：1:横(G-A), 2:竖(H-M), 3:撇(T-Q), 4:点/捺(Y-P), 5:折(N-X)。
    *   **矩阵分布**：
        *   横 (1)：G F D S A
        *   竖 (2)：H J K L M
        *   撇 (3)：T R E W Q
        *   点 (4)：Y U I O P
        *   折 (5)：N B V C X
*   **取码规则**：
    *   **码 1**：对应汉字的 **前 2 笔**。
    *   **码 2** (可选)：对应汉字的 **末 2 笔** (仅限 3 画及以上的字)。
*   **示例**：
    *   输入 `ren;v` ➔ **人** (撇3+捺4 -> `v`)。
    *   输入 `de;tu` ➔ **的** (前两笔“撇竖”=32=`r`（修正：此处应为矩阵坐标，如32=r），末两笔“点折”=45=`p` -> `rp`)。
    *   *(注：具体编码请参考 Web 端词库编辑器)*
*   **最强组合**：支持 `拼音;笔画英文` 混合过滤。
    *   例如：`li;mfC` 同时根据笔画 `mf` 和英文 `Ceremony` 过滤。
"""

content = re.sub(r'### 2\. 笔画辅助码.*?### 3\.', stroke_aux_new + '
### 3.', content, flags=re.DOTALL)

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)
print("README.md updated")
