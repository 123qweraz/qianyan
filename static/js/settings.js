let config = null;

async function loadConfig() {
    const r = await fetch('/api/config');
    config = await r.json();
    return config;
}

async function saveConfig() {
    await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config)
    });
    showToast("设置已保存并应用");
}

function showToast(message) {
    const toast = document.getElementById('toast');
    if (toast) {
        toast.innerText = message;
        toast.className = "status-toast show";
        setTimeout(() => { toast.className = "status-toast"; }, 2000);
    }
}

async function resetConfig() {
    if (confirm("确定要重置所有设置到默认值吗？")) {
        await fetch('/api/config/reset', { method: 'POST' });
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

    // 确定目标属性路径
    const path = propertyPath || id;
    const targetSection = section ? config[section] : config;
    
    let val = getNestedValue(targetSection, path);

    if (el.type === 'checkbox') {
        el.checked = !!val;
        el.onchange = () => {
            setNestedValue(targetSection, path, el.checked);
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
        if (el.tagName === 'SELECT') {
            el.onchange = update;
        }
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
