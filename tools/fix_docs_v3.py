import re
import os

def update_file(path, pattern, replacement):
    if not os.path.exists(path): return
    with open(path, 'r', encoding='utf-8') as f:
        content = f.read()
    new_content = re.sub(pattern, replacement, content, flags=re.S)
    with open(path, 'w', encoding='utf-8') as f:
        f.write(new_content)

# Update README.md
update_file('README.md', r'### 1\. 鑻辨枃杈呭姪鐮侊細.*?### 2\.', """### 1. 英文辅助码：义向筛选
杩欐槸 rust-ime 鐨勬牳蹇冪壒鑹层€傚綋鎷奸煶閲嶇爜杈冨鏃讹紝鏃犻渶缈婚〉锛屽彧闇€杈撳叆 **Shift + 澶у啓瀛楁瘝** 鍗冲彲鏍规嵁鈥滆嫳鏂囬噴涔夆€濈簿鍑嗗畾浣嶃€

*   **閫昏緫**锛氱郴缁熻嚜鍔ㄦ壂鎻忓€欓€夎瘝瀵瑰簲鐨勮嫳鏂囨敞閲婏紝鍖归厤棣栧瓧姣嶃€
*   **绀轰緥**锛
    *   杈撳叆 `li` \u2794 鍊欓€夛細閲屻€佺銆佺ぜ銆佺悊...
    *   杈撳叆 `liC` (Shift+C) \u2794 **绀** (Ceremony) 鐬棿鎺掑埌绗竴銆
    *   杈撳叆 `liL` (Shift+L) \u2794 **鐞** (Logic) 棿鎺掑埌绗竴銆
*   **骞跺瓨**锛氬彲涓庣瑪鐢昏緟鍔╃爜鍚屾椂浣跨敤锛堣涓嬫枃锛夈€

### 2.""")

update_file('README.md', r'### 2\. 绗旂敾杈呭姪鐮.*?### 3\.', """### 2. 笔画辅助码 (SBSRF 4码系统)：形向筛选
濡傛灉鎮ㄦ洿涔犳儻鏍规嵁姹夊瓧鐨勫舰鐘舵潵绛涢€夛紝鍙互浣跨敤鍩轰簬 **SBSRF (绗旂敾妫灄)** 閫昏緫鐨 4 鐮佽緟鍔╃郴缁熴€

*   **杈撳叆鏂瑰紡**锛氬湪鎷奸煶鍚庤緭鍏 **鍒嗗彿 `;`** 閿紝sui鍚庤緭鍏ョ瑪鐢绘槧灏勭爜锛堝皬鍐欏瓧姣嶏級銆
*   **甯冨眬閫昏緫 (5x5 鐭╅樀)**锛氬皢 5 绉嶅熀鏈瑪鐢绘槧灏勫埌 QWERTY 鍖哄煙锛
    *   **琛**锛氫唬琛ㄨ瀵圭瑪鐢讳腑鐨 **绗 1 绗**銆
    *   **鍒**锛氫唬琛ㄨ瀵圭瑪鐢讳腑鐨 **绗 2 绗**銆
    *   **绗旂敾绱㈠紩**锛1:妯(G-A), 2:绔(H-M), 3:鎾(T-Q), 4:鎹/鐐(Y-P), 5:鎶(N-X)銆
    *   **鐭╅樀鍒嗗竷**锛
        *   妯 (1)锛欸 F D S A
        *   绔 (2)锛欻 J K L M
        *   鎾 (3)锛歍 R E W Q
        *   鎹 (4)锛Y U I O P
        *   鎶 (5)锛N B V C X
*   **鍙栫爜瑙勫垯**锛
    *   **鐮 1**锛氬搴旀眽瀛楃殑 **鍓 2 绗**銆
    *   **鐮 2** (鍙€)锛氬搴旀眽瀛楃殑 **鏈 2 绗** (浠呴檺 3 鐢诲強浠ヤ笂鐨勫瓧)銆
*   **绀轰緥**锛
    *   杈撳叆 `ren;v` \u2794 **浜** (鎾3+鎹4 -> `v`)銆
    *   杈撳叆 `de;rp` \u2794 **鐨** (鍓嶄袱绗斺€滄拠绔栤€=32=`r`锛屾湯涓ょ瑪鈥滅偣鎶樷€=45=`p` -> `rp`)銆
*   **鏈€寮虹粍鍚**锛氭敮鎸 `鎷奸煶;绗旂敾鑻辨枃` 娣峰悎杩囨护銆
    *   渚嬪锛li;mfC` 鍚屾椂鏍规嵁绗旂敾 `mf` 鍜岃嫳鏂 `Ceremony` 杩囨护銆

### 3.""")

# Update help.html
if os.path.exists('static/help.html'):
    with open('static/help.html', 'r', encoding='utf-8') as f:
        h = f.read()
    h = h.replace('\u53ea\u9700\u8f93\u5165<b>\u5927\u5199\u5b57\u6bcd</b>', '\u53ea\u9700\u8f93\u5165<b>Shift + \u5927\u5199\u5b57\u6bcd</b>')
    # Use simpler regex for help.html
    h = re.sub(r'<h3>2\. \u7f16\u7801\u89c4\u5219</h3>.*?</div>\s*</div>\s*</section>', """<h3>2. \u7f16\u71\u89c4\u5219</h3>
                <ul>
                    <li><b>\u5206\u9694\u7b26</b>\uff1a\u5728\u62fc\u97f3\u540e\u8f93\u5165<b>\u5206\u53f7 ;</b> \u952e\u8fdb\u5165\u7b14\u753b\u8fc7\u6ee4\u6a21\u5f0f\u3002</li>
                    <li><b>\u524d 2 \u7b14 (\u7b2c 1 \u952e)</b>\uff1a\u6c49\u5b57\u7684\u524d\u4e24\u4e2a\u7b14\u753b\u3002\u4f8b\u5982\u201c\u7684\u201d\u524d\u4e24\u7b14\u662f\u6487\u3001\u7ad6\uff0c\u5bf9\u5e14 <code>r</code>\u3002</li>
                    <li><b>\u672b 2 \u7b14 (\u7b2c 2 \u952e)</b>\uff1a\u6c49\u5b57\u7684\u6700\u540e\u4e24\u4e2a\u7b14\u753b\uff083\u753b\u53ca\u4ee5\u4e0a\u6709\u6548\uff09\u3002\u4f8b\u5982\u201c\u7684\u201d\u672b\u4e24\u7b14\u662f\u70b9\u3001\u6298\uff0c\u5bf9\u5e14 <code>p</code>\u3002</li>
                </ul>

                <div class="example-box">
                    <p><b>\u5b9e\u6218\u6f14\u7ec3</b></p>
                    <ul>
                        <li><b>\u4eba</b> (ren)\uff1a\u64873+\u637a4 \u2794 \u8f93\u5165 <code>ren;v</code> \u77ac\u95f4\u5b9a\u4f4d\u3002</li>
                        <li><b>\u7684</b> (de)\uff1a\u524d\u4e24\u7b14\u201c\u6487\u7ad6\u201d=32=<code>r</code>\uff0c\u672b\u4e24\u7b14\u201c\u70b9\u6298\u201d=45=<code>p</code> \u2794 \u8f93\u5165 <code>de;rp</code> \u7cbe\u51c6\u5b9a\u4f4d\u3002</li>
                        <li><b>\u6df7\u5408\u4f7f\u7528</b>\uff1a\u8f93\u5165 <code>li;mfC</code> \u540c\u65f6\u6309\u7b14\u753b <code>mf</code> \u548c\u82f1\u6587 <code>Ceremony</code> \u8fc7\u6ee4\u3002</li>
                    </ul>
                </div>
            </div>
        </section>""" , h, flags=re.S)
    with open('static/help.html', 'w', encoding='utf-8') as f:
        f.write(h)

print("Docs updated successfully")
