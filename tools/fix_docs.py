import re
import os

def update_readme():
    file_path = 'README.md'
    if not os.path.exists(file_path): return
    content = open(file_path, 'r', encoding='utf-8').read()

    # Update English Aux
    en_pattern = r'### 1\. 英文辅助码：义向筛选.*?### 2\.'
    en_new = """### 1. 英文辅助码：义向筛选
这是 rust-ime 的核心特色。当拼音重码较多时，无需翻页，只需输入 **Shift + 大写字母** 即可根据“英文释义”精准定位。

*   **逻辑**：系统扫描候选词的英文注释，匹配首字母。
*   **示例**：
    *   输入 `li` ➔ 候选：里、离、礼、理...
    *   输入 `liC` (Shift+C) ➔ **礼** (Ceremony) 瞬间排到第一。
    *   输入 `liL` (Shift+L) ➔ **理** (Logic) 瞬间排到第一。
*   **并存**：可与笔画辅助码同时使用（见下文）。
"""
    content = re.sub(en_pattern, en_new + "
### 2.", content, flags=re.DOTALL)

    # Update Stroke Aux
    stroke_pattern = r'### 2\. 笔画辅助码.*?### 3\.'
    stroke_new = """### 2. 笔画辅助码 (SBSRF 4码系统)：形向筛选
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
    *   输入 `de;tu` ➔ **的** (前两笔“撇竖”=32=`r`，末两笔“点折”=45=`p` -> `rp`)。
*   **最强组合**：支持 `拼音;笔画英文` 混合过滤。
    *   例如：`li;mfC` 同时根据笔画 `mf` 和英文 `Ceremony` 过滤。
"""
    content = re.sub(stroke_pattern, stroke_new + "
### 3.", content, flags=re.DOTALL)

    with open(file_path, 'w', encoding='utf-8') as f:
        f.write(content)
    print("README.md updated.")

def update_help_html():
    file_path = 'static/help.html'
    if not os.path.exists(file_path): return
    content = open(file_path, 'r', encoding='utf-8').read()

    # Update stroke rules in help.html
    rules_pattern = r'<h3>2\. 编码规则</h3>.*?</div>'
    rules_new = """<h3>2. 编码规则</h3>
                <ul>
                    <li><b>分隔符</b>：在拼音后输入<b>分号 ;</b> 键进入笔画过滤模式。</li>
                    <li><b>前 2 笔 (第 1 键)</b>：汉字的前两个笔画。例如“的”前两笔是撇、竖，对应 <code>r</code>。</li>
                    <li><b>末 2 笔 (第 2 键)</b>：汉字的最后两个笔画（3画及以上有效）。例如“的”末两笔是点、折，对应 <code>p</code>。</li>
                </ul>

                <div class="example-box">
                    <p><b>实战演练</b></p>
                    <ul>
                        <li><b>人</b> (ren)：撇3+捺4 ➔ 输入 <code>ren;v</code> 瞬间定位。</li>
                        <li><b>的</b> (de)：前两笔“撇竖”=32=<code>r</code>，末两笔“点折”=45=<code>p</code> ➔ 输入 <code>de;rp</code> 精准定位。</li>
                        <li><b>混合使用</b>：输入 <code>li;mfC</code> 同时按笔画 <code>mf</code> 和英文 <code>Ceremony</code> 过滤。</li>
                    </ul>
                </div>"""
    
    # Use re.DOTALL to match across lines, and find the specific section after id="stroke-aux"
    section_start = content.find('id="stroke-aux"')
    if section_start != -1:
        part = content[section_start:]
        new_part = re.sub(r'<h3>2\..*?</ul>\s*<div class="example-box">.*?</div>', rules_new, part, flags=re.DOTALL)
        content = content[:section_start] + new_part

    # Update English aux to mention Shift
    content = content.replace('只需输入<b>大写字母</b>', '只需输入<b>Shift + 大写字母</b>')

    with open(file_path, 'w', encoding='utf-8') as f:
        f.write(content)
    print("help.html updated.")

if __name__ == "__main__":
    update_readme()
    update_help_html()
