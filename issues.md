# 待修复问题

## 高优先级

1. **icon.rc 图标路径错误**
   - 文件：`icon.rc:2`
   - 内容：`1 ICON "picture/rust-ime_v2.ico"`
   - 问题：实际文件为 `picture/qianyan-ime_v2.ico`，Windows 构建会失败

2. **platform-windows/main.rs 引用不存在的模块**
   - 文件：`crates/platform-windows/src/main.rs:10-11`
   - 内容：`pub mod tray;` `pub mod runtime;`
   - 问题：`tray.rs` 和 `runtime.rs` 均不存在，该二进制文件无法编译

## 中优先级

3. **.gitignore 忽略了 Cargo.lock**
   - 文件：`.gitignore:3`
   - 问题：二进制应用项目应提交 `Cargo.lock` 以保证可重现构建

4. **安装脚本引用不存在的 fonts/ 目录**
   - 文件：`qianyan-ime.iss:37`
   - 内容：`Source: "fonts\*"; DestDir: "{app}\fonts";`
   - 问题：项目根目录不存在 `fonts/` 目录

## 低优先级

5. **platform-linux/mod.rs 冗余模块声明**
   - 文件：`crates/platform-linux/src/mod.rs`
   - 问题：`lib.rs` 已声明 `cli` 和 `runtime` 模块，`mod.rs` 重复声明

6. **AGENTS.md 空文件**
   - 文件：`AGENTS.md`（0 bytes）

7. **30 个 clippy 警告**
   - type_complexity (6), let_underscore_future (5), too_many_arguments (2), collapsible_if (2), 其他 (15)
