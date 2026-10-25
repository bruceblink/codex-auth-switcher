# 第一版开发计划

> 历史文档。第一版 Python 源码已从当前分支删除。此计划仅记录原始范围，不代表当前项目仍维护 Python。

## 目标

在 Windows 上提供可运行的 GUI 和 CLI，成组切换 Codex 的模式配置与认证。第一版保留 Python + Tkinter，不执行框架迁移。

## 范围

1. 读取当前主配置和已有备份，不依赖 `autocode.config.toml`。
2. 保存两种模式的 provider、模型、推理级别、上下文参数和认证。
3. 保留当前共享设置。认证存档通过 Windows DPAPI 加密。
4. GUI 支持输入 provider、base_url、脱敏 key，以及导入模式来源。UI 输入优先于环境变量。
5. 配置先解析校验再备份。文件更新失败时恢复，外部冲突时停止。
6. 提供隔离目录测试、真实窗口验收和源码启动说明。

## 接受条件

- API 与订阅往返可以恢复各自的模型参数和认证。
- 缺失认证、无效 TOML、错误 URL、模型推理级别不支持时不写入活动文件。
- 配置备份不覆盖已有文件，认证不创建新的明文备份。
- API key 不进入项目设置、日志或配置。输出不包含认证正文。
- 现有共享配置、`autocode.config.toml` 和已有认证备份不被覆盖。
- 明确提示旧会话 provider 可能不变。远程认证与旧会话迁移分别记录为未验证和未实现。

## 验证命令

```powershell
py -3 -m unittest discover -s tests -v
py -3 -m compileall -q main.py src tests
py -3 main.py --target status
py -3 tests/gui_acceptance.py
```

不适用：Rust 检查、Docker 集成测试、远程 API 测试。

## 回滚边界

代码改动限于本项目。真实 Codex 目录只初始化加密状态，不在当前会话执行模式切换。认证恢复记录保存在 Codex 目录，不进入仓库。详细边界见 [认证模式恢复计划](02-auth-mode-recovery-plan.md)。
