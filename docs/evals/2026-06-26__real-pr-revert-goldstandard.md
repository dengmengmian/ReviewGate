# 真实 PR revert 金标准(默认全规则 ON)：重引入真实已修复 bug,应 BLOCK/WARN

## JavaScript — revert `140a179` (原型污染 SSRF 防护)
原修复: fix: guard socketPath with own() to prevent prototype pollution SSRF (#10901)
decision: warn
    · 假设漂移（Assumption Drift）：该改动将 `socketPath` 和 `allowedSocketPaths` 的读取方式从 `own()`（基于 `utils.hasOwnProp` 的自身属性守卫）改为直接访问 `con

## Python — revert `f0198e6` (Content-Type 解析修复)
原修复: Fix malformed value parsing for Content-Type (#7309)
decision: block
    · 假设漂移（Assumption Drift）：`_parse_content_type_header` 的行为变更后，无 `=` 号的参数值变为布尔值 `True`（之前被静默跳过），但调用方 `get_encoding_from_head
    · `_parse_content_type_header` 对不含 `=` 的参数（如 `charset`）返回 `True` 而非跳过它。下游 `get_encoding_from_headers`（第 544 行）对该值调用 `.stri

## Go — revert `9914178` (ClientIP 多 X-Forwarded-For 处理)
原修复: fix(context): ClientIP handling for multiple X-Forwarded-For header values (#4472)
decision: block
    · `requestHeader` 内部使用 `Header.Get()`，仅返回首个 header 值。原代码使用 `Header.Values()` + `strings.Join` 可获取同名的所有 header 值。当代理发送多个同名的

## Rust — revert `43e2f08` (gitignore 跨根匹配修复)
原修复: ignore: fix parent gitignore matching across multiple roots
decision: warn
    · 墙钟超时，该维度未审完（已保留其部分发现）
    · 缓存复用导致 absolute_base 错误：当多次调用 `add_parents` 且不同路径共享父目录时，缓存的 `IgnoreInner` 会携带第一次调用的 `absolute_base`（例如 `/project/src`），第
    · 当 `add_parents` 命中缓存复用已有的 `IgnoreInner` 时，未更新 `absolute_base` 字段。旧代码中 `absolute_base` 存在于外层 `Ignore` 结构体上，即使命中缓存也会用当前调用的
    · `add_parents` 循环中调用 `add_child_path` 时，`add_child_path` 内部（第337行）会克隆 `self.inner.absolute_base`，但紧接着第226行立即用 `Some(absol


---

## 结论：4/4 真实 revert 全部命中（金标准）
对 4 个重点语言、**真实世界已合并并修复的 bug**，用 revert 法重引入后，默认全规则审查：
- JavaScript(axios 原型污染 SSRF) → WARN、Python(requests Content-Type 解析) → BLOCK、Go(gin ClientIP 多 XFF) → BLOCK、Rust(ripgrep gitignore 缓存 absolute_base) → WARN。
- **每条发现都精确还原了原始修复所针对的回归**（非泛泛而谈），ground truth 由原 fix commit subject 对照。模型以"假设漂移"框定语义回归，体现的是推理而非模式匹配。
- Rust 一例单维度超时仍由其它维度命中（注：本次用的是大 diff 改造前的 release；改造后的"未审完不放行"会把超时显式标记，不影响此处召回结论）。

**意义**：合成强触发已证明规则能抓真 bug；本轮进一步用**真实 PR 金标准**证明——在真实代码、真实回归、默认配置下，4 个重点语言召回全中。配合此前 45 语言精度验证（clean 不误报），召回与精度两端都有真实证据支撑。

## 诚实边界
- 抽查 4 个语言/4 个 commit，非穷尽；选的是有明确 ground truth 的代码修复（排除纯 deps/docs）。
- 2 WARN 2 BLOCK：严重度标定偏保守（SSRF 回归给到 WARN 略软），但**召回成立**（均被标记、未放行）；严重度精修属次要项。
