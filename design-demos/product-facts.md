# 事实核对

- 本地权威：/home/ferychen/Workspace/devtray，Cargo版本0.1.1；资源及Skills功能未发布。
- Agent检测当前只检查PATH，版本仅读stdout；Pi可位于~/bin，因此桌面PATH漏检已复现。
- 资源扫描输出相对路径，_clean-plan的--path按绝对路径解析；传node_modules稳定报“解析失败”。
- Agent目前无远程检查实现；Skills按GitHub tree sha检查，并非semver，也非每种来源通用。
- 外部核对2026-10-29：Fork官方 https://fork.dev/ ；Raycast官方 https://www.raycast.com/ ；Skills CLI官方 https://github.com/vercel-labs/skills/blob/main/docs/commands.md 。只借桌面布局，不引用其版本/价格。
- 原型不声称实时运行状态。目录数据来自缓存；演示活动和告警标明演示。
