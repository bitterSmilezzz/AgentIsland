# R19 验证记录：工具链与发布强化（v0.0.36）

日期：2026-09-12 · 执行：主 agent

## 改动
- build-app.sh：版本从 CHANGELOG 自动抽取（$1 覆盖）+ README 漂移拒绝 + 测试门禁（SKIP_TESTS=1 跳过）
- test-token-layout.py：stderr 可见 + swift build 新鲜度前置
- ADR-0003 决策 4 状态注记；docs/validation/ 入库（R01–R19 全部记录）

## 负例与门禁实证
1. README 版本漂移（0.0.99）→ 打包被拒（exit 1，报错指明两处版本）
2. 测试门禁首跑即抓到 R19 自身哨兵失效（脚本去硬编码后 <string> 正则过时）——门禁按设计工作，哨兵语义更新为「抽取逻辑存在 + 无硬编码残留」
3. bash 多字节变量名陷阱：$VERSION（ 后跟全角字符被并入变量名（set -u 报 unbound）——${VERSION} 花括号修复
4. 最终：脚本自动抽取 0.0.36 打包成功 + 门禁 180/0 + selftest 全过

## 经验
- 发布流程从「手工同步 4 处版本」简化为「CHANGELOG 写新条目 + README 功能版本 + ./scripts/build-app.sh」
- R20 起版本管理：CHANGELOG/README/version-mapping 三处仍需手工（build-app.sh 已自动化）
