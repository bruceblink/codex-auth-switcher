use switcher_core::Mode;

pub(super) fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

pub(super) fn mode_name(mode: Mode) -> &'static str {
    if mode == Mode::Api {
        "API key"
    } else {
        "订阅"
    }
}

pub(super) fn auth_source_name(source: &str) -> &str {
    match source {
        "user input" => "界面输入",
        "environment" => "旧存档环境来源",
        "latest active authentication" => "当前活动认证",
        source => source,
    }
}

pub(super) fn masked_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    if chars.len() < 10 {
        return "已输入密钥".into();
    }
    format!(
        "{}······{}",
        chars.iter().take(4).collect::<String>(),
        chars.iter().skip(chars.len() - 4).collect::<String>()
    )
}

pub(super) fn user_error(error: &switcher_core::Error) -> String {
    let message = match error.0 {
        "Encryption key is missing. Restore vault.key; existing ciphertext was not changed." => {
            "本地加密密钥缺失。请恢复 vault.key；工具未修改现有密文。"
        }
        "Cannot authenticate encrypted state. Check the key and ciphertext." => {
            "加密存档验证失败。请检查 vault.key 与密文；工具未修改现有存档。"
        }
        "Invalid local encryption key." | "Invalid encryption key." => {
            "本地加密密钥无效。请恢复正确的 vault.key。"
        }
        "Invalid encrypted state format or size." => "加密存档格式无效或大小超出限制。",
        "Encrypted state exceeds the size limit." => "加密存档超过大小限制。",
        "State encryption failed." => "本地状态加密失败。",
        "Cannot restrict local state permissions. Use a private NTFS user directory." => {
            "无法限制本地存档目录的访问权限。请使用仅当前用户可访问的 NTFS 目录。"
        }
        "Cannot restrict state directory permissions." => "无法设置本地存档目录权限。",
        "Cannot create the user state directory." => "无法创建本地存档目录。",
        "State directory must be a regular directory." => "本地存档路径必须是目录。",
        "Another switcher operation is running." => "已有切换操作正在运行，请稍后重试。",
        "Cannot open operation lock." => "无法取得切换操作锁，请检查目录权限。",
        "Version one has a pending write. Recover it with version one before continuing." => {
            "检测到第一版未完成的写入。请使用历史版本恢复后再继续。"
        }
        "Invalid encrypted recovery record." => "加密恢复记录无效，工具已停止恢复。",
        "Unsupported recovery record version." => "恢复记录版本不受支持。",
        "Mode vault is missing during recovery." => "恢复时缺少模式密钥库，请先恢复 vault.key。",
        "External changes block recovery. Preserve pending.enc and resolve the conflict." => {
            "检测到外部文件改动，恢复已停止。请保留 pending.enc 并先解决冲突。"
        }
        "Cannot remove completed recovery record." => "无法清除已完成的恢复记录。",
        "Cannot restore missing authentication state." => "无法恢复缺失的认证状态。",
        "Recovery is pending. Initialize modes to recover before previewing." => {
            "检测到未完成的恢复记录。请先执行初始化以恢复，再生成预览。"
        }
        "The preview belongs to a different directory or recovery is pending." => {
            "此预览对应其他配置目录，或存在未完成的恢复记录。请刷新并重新预览。"
        }
        "Files changed since preview. Refresh and preview again." => {
            "预览后文件已发生变化。请刷新并重新生成预览。"
        }
        "Invalid API base URL." => "Base URL 格式无效，请检查协议和主机名。",
        "API base URL must be HTTP(S) with a host and no credentials, query or fragment." => {
            "Base URL 必须使用 HTTP 或 HTTPS，并包含主机名；不能包含认证信息、查询参数或片段。"
        }
        "API key is missing. Enter a key before switching." => "缺少 API key。请输入密钥后再切换。",
        "API key is empty, too long or contains whitespace." => {
            "API key 为空、过长或包含空白字符。请检查输入。"
        }
        "Target authentication is missing. Import a matching auth file." => {
            "目标模式缺少认证。请导入对应模式的认证 JSON。"
        }
        "Current authentication is missing. Import valid authentication first." => {
            "当前活动认证缺失。请先导入有效认证。"
        }
        "Invalid TOML configuration. Original files were not changed." => {
            "TOML 配置无效；原文件未修改。"
        }
        "Invalid authentication JSON. Values are hidden." => "认证 JSON 无效。认证值已隐藏。",
        "Switch failed; previous configuration and authentication were restored." => {
            "切换失败。工具已恢复切换前的配置与认证。"
        }
        "Switch incomplete. Preserve pending.enc, close Codex and initialize to recover." => {
            "切换未完成。请保留 pending.enc，关闭 Codex 并执行初始化以恢复。"
        }
        message => return format!("操作失败：{message}"),
    };
    message.to_string()
}
