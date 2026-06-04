(function() {
    if (document.getElementById('qy-sidebar')) return;

    var sidebarHTML = '<aside id="qy-sidebar" class="qy-sidebar">' +
        '<div class="qy-sidebar-logo"><h2>千言输入法</h2><span class="version">v0.1.0</span></div>' +
        '<div class="qy-sidebar-search"><input type="text" id="qy-sidebar-search-input" placeholder="搜索..." oninput="filterSidebar()"></div>' +
        '<nav class="qy-sidebar-nav" id="qy-sidebar-nav">' +
        '<a href="#section-settings" data-keywords="系统 拼音 候选栏 模糊音 双拼 快速 CapsLock 外观 键盘布局">⚙️ 设置部分</a>' +
        '<a href="#section-dicts" data-keywords="词典 词库 用户词典 自造词 新词 编译 编辑">📖 词典工具</a>' +
        '<a href="#section-learning" data-keywords="学习 练习 笔画码 文章 打字 辅助码">✍️ 学习部分</a>' +
        '<a href="#section-tools" data-keywords="工具 emoji 表情 符号 转换 虚拟键盘 网页输入 大写 日期">🧰 额外功能</a>' +
        '<a href="#section-help" data-keywords="帮助 备份 恢复 导入 导出 迁移 还原">📦 帮助与备份</a>' +
        '</nav>' +
        '</aside>';

    document.body.insertAdjacentHTML('afterbegin', sidebarHTML);

    document.querySelectorAll('.qy-sidebar-nav a').forEach(function(a) {
        a.addEventListener('click', function(e) {
            e.preventDefault();
            var target = document.querySelector(a.getAttribute('href'));
            if (target) {
                target.scrollIntoView({ behavior: 'smooth', block: 'start' });
            }
        });
    });

    if (!document.getElementById('qy-main-wrap')) {
        var wrap = document.createElement('div');
        wrap.id = 'qy-main-wrap';
        wrap.className = 'qy-main-content';
        while (document.body.children.length > 1) {
            var child = document.body.children[1];
            if (child.id === 'toast' || child.tagName === 'SCRIPT') break;
            wrap.appendChild(child);
        }
        document.body.appendChild(wrap);
    }

    var headings = document.querySelectorAll('h2[id^="section-"]');
    if (headings.length > 0) {
        var observer = new IntersectionObserver(function(entries) {
            entries.forEach(function(entry) {
                if (entry.isIntersecting) {
                    document.querySelectorAll('.qy-sidebar-nav a').forEach(function(a) {
                        a.classList.toggle('active', a.getAttribute('href') === '#' + entry.target.id);
                    });
                }
            });
        }, { rootMargin: '-80px 0px -60% 0px' });
        headings.forEach(function(h) { observer.observe(h); });
    }
})();

function filterSidebar() {
    var q = document.getElementById('qy-sidebar-search-input').value.toLowerCase();
    document.querySelectorAll('.qy-sidebar-nav a').forEach(function(a) {
        var kw = (a.dataset.keywords || '') + ' ' + a.textContent;
        a.style.display = kw.toLowerCase().indexOf(q) >= 0 ? '' : 'none';
    });
}
