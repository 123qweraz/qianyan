(function() {
    if (document.getElementById('qy-sidebar')) return;

    const sidebarHTML = `
<aside id="qy-sidebar" class="qy-sidebar">
    <div class="qy-sidebar-logo">
        <h2>千言输入法</h2>
        <span class="version">v0.1.0</span>
    </div>
    <div class="qy-sidebar-search">
        <input type="text" id="qy-sidebar-search-input" placeholder="搜索设置..." oninput="filterSidebar()">
    </div>
    <nav class="qy-sidebar-nav" id="qy-sidebar-nav">
        <div class="qy-nav-group">
            <div class="qy-nav-group-title">设置</div>
            <a href="/static/settings/system.html" data-keywords="系统 启动 路径 linux 编辑器">💻 系统设置</a>
            <a href="/static/settings/pinyin.html" data-keywords="拼音 快捷键 按键 繁体 输入法">🔤 拼音设置</a>
            <a href="/static/settings/appearance.html" data-keywords="候选栏 颜色 字体 外观 皮肤">🎨 候选栏设置</a>
            <a href="/static/settings/fuzzy.html" data-keywords="模糊音 z zh s sh in ing">☁️ 模糊音设置</a>
            <a href="/static/settings/doublepinyin.html" data-keywords="双拼 自然码 小鹤 键位">🧬 双拼方案</a>
            <a href="/static/settings/quickfinals.html" data-keywords="长韵母 快速 CapsLock 映射">⌨️ 快速输入</a>
            <a href="/static/settings/punctuation.html" data-keywords="标点 符号 映射 键位">📝 标点符号</a>
            <a href="/static/settings/layout.html" data-keywords="键盘布局 方案 中文 英文 日语">⌨️ 键盘布局</a>
            <a href="/static/settings/hotkeys.html" data-keywords="热键 快捷键 切换 组合键">⚡ 快捷键</a>
            <a href="/static/settings/keybehavior.html" data-keywords="按键行为 组合 CapsLock 数字 换行">🔑 按键行为</a>
        </div>
        <div class="qy-nav-group">
            <div class="qy-nav-group-title">词典</div>
            <a href="/static/settings/dictionary.html" data-keywords="词典 启用 禁用 编译 切换">📚 词典设置</a>
            <a href="/static/settings/user_dict_editor.html" data-keywords="用户词典 自造词 频率 权重 学习">👤 用户词典</a>
            <a href="/static/settings/word_discovery.html" data-keywords="新词 发现 导入 导出 文本">🆕 新词发现</a>
            <a href="/static/settings/dictionary_editor.html" data-keywords="编辑 浏览 词条 添加 删除 修改">📖 词典编辑</a>
            <a href="/static/settings/dictionary_viewer.html" data-keywords="辅助码 笔画 对照表 查看">📋 辅助码</a>
        </div>
        <div class="qy-nav-group">
            <div class="qy-nav-group-title">学习</div>
            <a href="/static/settings/learning.html" data-keywords="练习 学习 汉字 词组 词典">📖 练习学习</a>
            <a href="/static/settings/stroke_practice.html" data-keywords="笔画码 数字 键位 专项 训练">📏 笔画练习</a>
            <a href="/static/settings/article_practice.html" data-keywords="文章 打字 速度 千字文 导入">📝 文章练习</a>
        </div>
        <div class="qy-nav-group">
            <div class="qy-nav-group-title">工具</div>
            <a href="/static/settings/emojis.html" data-keywords="emoji 表情 图标 符号">😄 表情选择</a>
            <a href="/static/settings/symbols.html" data-keywords="符号 标点 特殊字符 希腊 俄语">✨ 符号选择</a>
            <a href="/static/settings/converter.html" data-keywords="转换 简体 繁体 拼音 分词">🔄 转换器</a>
            <a href="/static/settings/pinyin_converter.html" data-keywords="拼音 转拼音 中文转拼音 汉字转拼音">🔠 拼音转换</a>
            <a href="/static/settings/utilities.html" data-keywords="工具 数字 大写 金额 日期 时间">🧰 实用工具</a>
            <a href="/static/virtual_keyboard.html" data-keywords="网页 虚拟键盘 测试 输入法 引擎">⌨️ 网页输入</a>
            <a href="/static/help.html" data-keywords="帮助 指南 操作 问题 快捷键">📖 帮助</a>
        </div>
        <div class="qy-nav-group">
            <div class="qy-nav-group-title">备份</div>
            <a href="/static/settings/backup.html" data-keywords="备份 导出 导入 恢复 还原 全部数据 配置">📦 备份恢复</a>
        </div>
    </nav>
</aside>`;

    document.body.insertAdjacentHTML('afterbegin', sidebarHTML);

    // 高亮当前页
    const current = window.location.pathname;
    document.querySelectorAll('.qy-sidebar-nav a').forEach(a => {
        if (current === new URL(a.href).pathname) {
            a.classList.add('active');
        }
    });

    // 包裹原有内容
    const container = document.querySelector('body > .container, body > :not(#qy-sidebar):not(script)');
    if (!document.getElementById('qy-main-wrap')) {
        const wrap = document.createElement('div');
        wrap.id = 'qy-main-wrap';
        wrap.className = 'qy-main-content';
        while (document.body.children.length > 1) {
            const child = document.body.children[1]; // after sidebar
            if (child.tagName === 'SCRIPT') {
                // 跳过脚本标签
                let next = child.nextElementSibling;
                if (next && next.tagName !== 'SCRIPT') {
                    wrap.appendChild(next);
                }
                break;
            }
            if (child.id === 'toast' || child.id === 'qy-sidebar') {
                break;
            }
            wrap.appendChild(child);
        }
        document.body.appendChild(wrap);
    }
})();

function filterSidebar() {
    const q = document.getElementById('qy-sidebar-search-input').value.toLowerCase();
    document.querySelectorAll('.qy-sidebar-nav a').forEach(a => {
        const kw = (a.dataset.keywords || '') + ' ' + a.textContent;
        a.style.display = kw.toLowerCase().includes(q) ? '' : 'none';
    });
    // 显示/隐藏分组标题
    document.querySelectorAll('.qy-nav-group').forEach(g => {
        const visible = g.querySelectorAll('a[style*="display: none"]').length < g.querySelectorAll('a').length;
        g.querySelector('.qy-nav-group-title').style.display = visible ? '' : 'none';
    });
}
