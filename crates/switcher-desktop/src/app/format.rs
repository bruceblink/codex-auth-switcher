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

pub(super) fn config_source_name(source: &str) -> &str {
    match source {
        "latest active config" => "当前活动配置",
        "default API parameters" => "官方默认 API 参数",
        source => source,
    }
}

pub(super) fn config_field_name(field: &str) -> &str {
    match field {
        "model_provider" => "Provider",
        "model" => "模型",
        "model_reasoning_effort" => "推理级别",
        "plan_mode_reasoning_effort" => "计划模式推理级别",
        "model_reasoning_summary" => "推理摘要",
        "model_supports_reasoning_summaries" => "支持推理摘要",
        "model_context_window" => "上下文窗口",
        "model_auto_compact_token_limit" => "自动压缩阈值",
        "model_catalog_json" => "模型目录",
        "disable_response_storage" => "禁用响应存储",
        "service_tier" => "服务层级",
        "personality" => "个性",
        "forced_login_method" => "登录方式限制",
        "forced_chatgpt_workspace_id" => "工作区限制",
        "model_verbosity" => "模型详细度",
        "model_auto_compact_token_limit_scope" => "压缩阈值范围",
        "model_post_turn_compact_threshold_percent" => "回合后压缩阈值",
        "model_instructions_file" => "模型指令文件",
        "review_model" => "审查模型",
        "chatgpt_base_url" => "ChatGPT 地址",
        "oss_provider" => "OSS provider",
        other => other,
    }
}

pub(super) fn setting_value(value: &str) -> &str {
    match value {
        "(default)" => "默认",
        "(configured)" => "已配置",
        "configured" => "已配置",
        "not set" => "未设置",
        value => value,
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
        "A custom API base URL requires a custom provider ID. The built-in openai provider uses the official endpoint." => {
            "第三方 API 地址需要自定义 Provider ID。内置 openai 只使用官方地址。"
        }
        "Current authentication is invalid. Import valid authentication first." => {
            "当前认证无效。请先在 Codex 登录，或导入有效认证。"
        }
        "API key is empty, too long or contains whitespace." => {
            "API key 为空、过长或包含空白字符。请检查输入。"
        }
        "Codex models-codex.json was not found." => {
            "当前配置目录中没有 models-codex.json。请检查 Codex 目录。"
        }
        "Codex model catalog JSON is invalid." => "models-codex.json 格式无效，未修改文件。",
        "Codex model catalog must contain between 1 and 512 models." => {
            "模型目录必须包含 1 到 512 个模型。"
        }
        "Model IDs must be valid and unique." => "模型 ID 格式无效或重复。",
        "Model display names must be non-empty and at most 120 characters." => {
            "模型显示名称不能为空，且最多为 120 个字符。"
        }
        "Model visibility must be a short non-empty value." => {
            "模型可见性不能为空，且最多为 32 个字符。"
        }
        "Each model must have between 1 and 32 reasoning levels." => {
            "每个模型必须配置 1 到 32 个推理级别。"
        }
        "Model reasoning levels must be valid and unique." => "推理级别格式无效或重复。",
        "The default reasoning level must be supported by its model." => {
            "默认推理级别必须包含在该模型的支持列表中。"
        }
        "The model catalog belongs to another Codex directory." => {
            "此模型目录属于其他 Codex 配置目录。请重新读取。"
        }
        "The model catalog changed after it was loaded. Reload before saving." => {
            "读取后模型目录已被其他程序修改。请重新读取后再保存。"
        }
        "Cannot serialize the Codex model catalog." => "无法生成模型目录文件。",
        "Cannot reserve a unique model catalog backup." => "无法创建不覆盖的模型目录备份。",
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
        "The OpenAI Codex desktop app is not installed or could not be inspected." => {
            "未找到已安装的 Codex 桌面应用，或无法读取安装信息。请手动重启。"
        }
        "The OpenAI Codex desktop package information is invalid." => {
            "Codex 桌面应用安装信息无效。请手动重启。"
        }
        "Could not locate Windows PowerShell."
        | "Could not inspect the OpenAI Codex desktop installation."
        | "Timed out inspecting the OpenAI Codex desktop installation." => {
            "无法读取 Codex 桌面应用的安装信息。请手动重启。"
        }
        "Could not inspect Codex desktop windows."
        | "Could not inspect running Codex desktop processes." => {
            "无法检查 Codex 桌面窗口或进程。请手动重启。"
        }
        "Codex is running without an open desktop window. Exit it from the system tray, then retry." => {
            "Codex 正在后台运行且没有打开窗口。请从系统托盘退出后重试。"
        }
        "Could not ask Codex to close normally. Close it manually and retry."
        | "Could not confirm that Codex closed normally." => {
            "无法正常关闭 Codex。请保存工作后手动关闭并重新打开。"
        }
        "Codex did not exit within 15 seconds. Save your work, close it manually, and retry." => {
            "Codex 在 15 秒内没有退出。请保存工作后手动关闭并重新打开。"
        }
        "Windows could not launch the Codex desktop app."
        | "Codex did not open within 30 seconds. Open it manually and retry." => {
            "无法重新打开 Codex 桌面应用。请手动打开。"
        }
        message => return format!("操作失败：{message}"),
    };
    message.to_string()
}
