let config = null;

async function loadConfig() {
    const r = await fetch('/api/config');
    config = await r.json();
    return config;
}

async function saveConfig() {
    console.log('saveConfig: rare_char_mode=', config?.input?.rare_char_mode, 'full config keys:', Object.keys(config || {}));
    const resp = await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config)
    });
    if (!resp.ok) {
        console.error('saveConfig FAILED:', resp.status, resp.statusText);
        showToast("保存失败！请检查终端日志");
        return;
    }
    showToast("设置已保存并应用");
}

function showToast(message, type) {
    const toast = document.getElementById('toast');
    if (toast) {
        toast.textContent = message;
        toast.style.display = 'block';
        toast.style.background = type === 'error' ? '#ff3b30' : 'rgba(0,0,0,0.85)';
        toast.style.color = '#fff';
        clearTimeout(toast._timer);
        toast._timer = setTimeout(() => { toast.style.display = 'none'; }, 2500);
    }
}

async function resetConfig(section) {
    const msg = section
        ? "确定要重置当前页面的设置到默认值吗？"
        : "确定要重置所有设置到默认值吗？";
    if (confirm(msg)) {
        const url = section
            ? `/api/config/reset/${section}`
            : '/api/config/reset';
        await fetch(url, { method: 'POST' });
        location.reload();
    }
}

function getNestedValue(obj, path) {
    return path.split('.').reduce((prev, curr) => prev && prev[curr], obj);
}

function setNestedValue(obj, path, value) {
    const parts = path.split('.');
    const last = parts.pop();
    const target = parts.reduce((prev, curr) => prev && prev[curr], obj);
    if (target) target[last] = value;
}

function bindInput(id, section, propertyPath) {
    const el = document.getElementById(id);
    if (!el) return;

    // 确保配置对象存在
    if (!config) { console.warn('bindInput: config not loaded yet for', id); return; }

    // 确定目标属性路径
    const path = propertyPath || id;
    const targetSection = section ? config[section] : config;
    if (!targetSection) { console.warn('bindInput: targetSection missing for', id, 'section=', section); return; }
    
    let val = getNestedValue(targetSection, path);

    if (el.type === 'checkbox') {
        el.checked = !!val;
        el.onchange = () => {
            setNestedValue(targetSection, path, el.checked);
        };
    } else if (el.tagName === 'SELECT') {
        // select: 只有值有效时才设置，否则用第一个 option 的默认值
        if (val !== undefined && val !== null && val !== '') {
            el.value = val;
        } else {
            // 确保不出现空白选择，回退到第一个 option
            if (el.options.length > 0) {
                el.value = el.options[0].value;
                setNestedValue(targetSection, path, el.value);
            }
        }
        el.onchange = () => {
            setNestedValue(targetSection, path, el.value);
        };
    } else {
        el.value = (val !== undefined && val !== null) ? val : "";
        const update = () => {
            let newVal = el.value;
            if (el.type === 'number') {
                newVal = parseFloat(el.value);
                if (isNaN(newVal)) newVal = 0;
            }
            setNestedValue(targetSection, path, newVal);
        };
        el.oninput = update;
    }
}

function linkColor(id, section, propertyPath) {
    const txt = document.getElementById(id);
    const picker = document.getElementById(id + '_picker');
    if (!txt || !picker) return;

    const path = propertyPath || id;
    const targetSection = section ? config[section] : config;

    const toHex = (color) => {
        if (!color) return "#000000";
        if (color.startsWith('#')) return color.substring(0, 7);
        return "#000000";
    };

    // 初始化同步
    if (txt.value) {
        picker.value = toHex(txt.value);
    }

    txt.oninput = () => {
        picker.value = toHex(txt.value);
        setNestedValue(targetSection, path, txt.value);
    };
    picker.oninput = () => {
        txt.value = picker.value;
        setNestedValue(targetSection, path, picker.value);
    };
}

async function loadFonts(selectIds) {
    const font_r = await fetch('/api/fonts');
    const system_fonts = await font_r.json();
    const font_options = system_fonts.map(f => `<option value="${f.name}">${f.name}</option>`).join('');
    
    selectIds.forEach(id => {
        const select = document.getElementById(id);
        if (select) {
            const currentVal = select.getAttribute('data-value');
            select.innerHTML = `<option value="">默认系统字体</option>` + font_options;
            if (currentVal) select.value = currentVal;
        }
    });
}

async function loadDictionaryViewer(filePath) {
    const tableBody = document.querySelector('#charsTable tbody');
    if (!tableBody) return;

    // 清空现有 DataTable
    if ($.fn.DataTable.isDataTable('#charsTable')) {
        $('#charsTable').DataTable().destroy();
    }

    tableBody.innerHTML = '<tr><td colspan="5" class="text-center py-5"><div class="spinner-border text-primary"></div></td></tr>';
    document.getElementById('fileInfo').innerText = '正在读取: ' + filePath;

    try {
        const url = filePath ? `/api/dictionary/chars?file=${encodeURIComponent(filePath)}` : '/api/dictionary/chars';
        const response = await fetch(url);
        const data = await response.json();
        
        let html = '';
        data.forEach(item => {
            html += `<tr class="pinyin-group-${item.group}">
                <td>${item.pinyin}</td>
                <td class="char-cell">${item.char}</td>
                <td><span class="aux-highlight-en">${item.en_aux}</span></td>
                <td><span class="aux-highlight-stroke">${item.stroke_aux}</span></td>
                <td>${item.en_meaning}</td>
            </tr>`;
        });
        
        tableBody.innerHTML = html;
        
        if (window.jQuery && $.fn.DataTable) {
            $('#charsTable').DataTable({
                pageLength: 25,
                language: {
                    search: "快速过滤：",
                    lengthMenu: "每页显示 _MENU_ 条",
                    info: "第 _START_ 到 _END_ 条，共 _TOTAL_ 条",
                    paginate: { first: "首页", last: "末页", next: "下一页", previous: "上一页" }
                },
                order: [[0, 'asc']]
            });
        }
    } catch (e) {
        tableBody.innerHTML = '<tr><td colspan="5" class="text-center text-danger py-5">数据加载失败: ' + e.message + '</td></tr>';
    }
}

// 关闭标签页时通知服务器退出，页面间导航时新页面的请求会在 3 秒内取消关闭
window.addEventListener('pagehide', () => {
    navigator.sendBeacon('/api/shutdown', '');
});
