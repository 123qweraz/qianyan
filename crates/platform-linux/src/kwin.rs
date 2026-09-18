use std::path::PathBuf;

pub fn is_kde_session() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|s| s.eq_ignore_ascii_case("kde"))
}

pub fn is_kwin_virtual_keyboard() -> bool {
    std::env::var("WAYLAND_SOCKET")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .is_some()
}

fn desktop_exec_value(command: &str) -> String {
    if command.contains(char::is_whitespace) {
        format!(
            "\"{}\"",
            command
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`")
        )
    } else {
        command.to_string()
    }
}

fn run_first_available_command(commands: &[&str], args: &[&str]) -> Result<(), String> {
    let mut errors = Vec::new();
    for command in commands {
        match std::process::Command::new(command).args(args).output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => errors.push(format!(
                "{command}: {} {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(e) => errors.push(format!("{command}: {e}")),
        }
    }
    Err(errors.join("; "))
}

fn ensure_kde_virtual_keyboard_desktop() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("无法确定 HOME 目录")?;
    let applications_dir = std::path::Path::new(&home)
        .join(".local")
        .join("share")
        .join("applications");
    std::fs::create_dir_all(&applications_dir)
        .map_err(|e| format!("创建 KDE desktop 目录失败: {e}"))?;

    let exe = std::env::current_exe()
        .map_err(|e| format!("无法获取当前可执行文件路径: {e}"))?;
    // 只对可执行文件路径做 quoting，参数必须留在引号之外：
    // KWin 用 KShell::splitArgs 解析 Exec，若把整条命令行包进引号会被当成
    // 单个可执行文件名（含空格）导致 QProcess 启动失败（且 KWin 静默忽略）。
    let exec_value = format!(
        "{} --backend=wayland",
        desktop_exec_value(&exe.display().to_string())
    );

    let desktop_file = applications_dir.join("qianyan-ime-wayland-launcher.desktop");
    let content = format!(
        "[Desktop Entry]\n\
         Name=Qianyan Input Method (Wayland)\n\
         Name[zh_CN]=千言输入法 (Wayland)\n\
         GenericName=Input Method\n\
         GenericName[zh_CN]=输入法\n\
         Comment=Qianyan Chinese Input Method Engine (KDE Virtual Keyboard)\n\
         Comment[zh_CN]=千言中文输入法引擎（KDE 虚拟键盘）\n\
         Exec={}\n\
         Icon=input-keyboard\n\
         Terminal=false\n\
         Type=Application\n\
         Categories=System;Utility;\n\
         StartupNotify=false\n\
         NoDisplay=true\n\
         OnlyShowIn=KDE\n\
         X-KDE-StartupNotify=false\n\
         X-KDE-Wayland-VirtualKeyboard=true\n",
        exec_value
    );
    std::fs::write(&desktop_file, content)
        .map_err(|e| format!("写入 KDE desktop 文件失败 {}: {e}", desktop_file.display()))?;
    Ok(desktop_file)
}

pub fn configure_kde_virtual_keyboard() -> Result<(), String> {
    if !is_kde_session() {
        return Err("当前不是 KDE 会话，无法配置 KDE 虚拟键盘输入法".into());
    }

    let desktop_file = ensure_kde_virtual_keyboard_desktop()?;
    // KWin 解析 kwinrc [Wayland] InputMethod 时用 readPathEntry -> KDesktopFile(路径)
    // 相对短名（如 "qianyan-ime-wayland-launcher.desktop"）无法解析，必须写绝对路径。
    let input_method_value = desktop_file.display().to_string();
    run_first_available_command(
        &["kwriteconfig6", "kwriteconfig5"],
        &[
            "--file",
            "kwinrc",
            "--group",
            "Wayland",
            "--key",
            "InputMethod",
            &input_method_value,
        ],
    )
    .map_err(|e| format!("写入 KDE 输入法配置失败: {e}"))?;

    log::info!(
        "[KWin] 虚拟键盘已注册: {} → kwinrc [Wayland] InputMethod",
        input_method_value
    );

    let _ = run_first_available_command(
        &["qdbus6", "qdbus"],
        &["org.kde.KWin", "/KWin", "reconfigure"],
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_value_leaves_plain_path_unquoted() {
        assert_eq!(desktop_exec_value("/usr/bin/qianyan-ime"), "/usr/bin/qianyan-ime");
    }

    #[test]
    fn exec_value_quotes_only_path_with_spaces() {
        assert_eq!(
            desktop_exec_value("/opt/my dir/qianyan-ime"),
            "\"/opt/my dir/qianyan-ime\""
        );
    }

    #[test]
    fn exec_line_arguments_stay_outside_quotes() {
        let exec_value = format!(
            "{} --backend=wayland",
            desktop_exec_value("/opt/my dir/qianyan-ime")
        );
        assert_eq!(exec_value, "\"/opt/my dir/qianyan-ime\" --backend=wayland");
        assert!(!exec_value.starts_with("\"/opt/my dir/qianyan-ime --backend=wayland\""));
    }
}
