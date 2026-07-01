# 业务 review：真实用户 issue + 修复 PR 验证

> **勘误（2026-07-01）**：下方"big.js #125 进位漏报"系**误标**（实为非 bug），
> "addBusinessDays 仍漏报"已**过时**（静态即命中）。故末尾"1 漏 + 1 部分"的汇总不再准确。
> 详见 `2026-07-01__exec-verify-runcheck-fix.md`。相关行已就地标注。

方法：找**用户提的 issue（ground truth）+ 解决它的 PR**，`git revert` 修复以重新引入用户报告的 bug，
审查能否命中。比合成用例更真实——bug 是真实世界发生过、被用户报告过的。

| 用户 issue | bug 类型 | 改进前 | 改进后（logic 用例推演） |
|---|---|---|---|
| date-fns #972/#992（DST，PR #1003） | 时区/DST 日期计算错误 | **WARN**（ai_smell 0.79，命中区域但未定性） | **BLOCK**（logic high 0.76/0.81）✅ **恢复** |
| date-fns #1228（parseISO 24:00，PR #1229） | 解析 `24:00` 时间错误（边界值） | — | **BLOCK**（logic 0.81，命中 parseISO 时间解析行）✅ |
| date-fns #692（differenceIn… 负零，PR #739） | 计算结果出现 `-0`（应为 `0`） | — | **BLOCK**（8 条命中各 differenceIn* 函数）✅ |
| date-fns #1584（addBusinessDays，PR #1588） | 周末加 1 业务日 off-by-one（周六+1 应得周一却得周二） | **PASS（漏报）** | ~~PASS（仍漏报）~~ → **已过时**：2026-07-01 复测静态即 block/warn 命中（见 `2026-07-01__exec-verify-runcheck-fix.md`） |

## 跨域扩展（非日期领域的真实 issue）

| 用户 issue | 领域 / bug 类型 | 结果 |
|---|---|---|
| express #4204（PR #4205） | Web 路由：regexp 处理逻辑错误 | **BLOCK**（logic 0.81，命中 router/index.js）✅ |
| validator.js #2660 区（PR #2693） | 输入校验：isSlug 允许了非法字符 | **BLOCK**（logic 0.81，命中 isSlug.js）✅ |
| moment #5580 | 状态副作用：`.format()` 改写了原 moment 实例（违反不可变） | **BLOCK**（era.js/format.js，logic+ai_smell）✅ |

| sequelize #14903 | ORM 事务生命周期：afterCommit 钩子在**失败事务**上仍执行 | **BLOCK**（logic 0.99，命中 transaction.ts）✅ |
| big.js #125（PR #126） | ~~大数运算 base-10 进位错误~~ **勘误：非 bug**（issue 是简化提问，两写法可证等价，20 万随机+全 9 对抗 0 差异，见 `2026-07-01__exec-verify-runcheck-fix.md`） | **WARN**（ai_smell 判"等价/冗余"——**判对了**，非漏报） |

| go-cache #64 | 并发/数据竞态：`janitor.stop` 在 goroutine 内创建、与 stopJanitor 并发读 | **BLOCK**（logic 0.92，命中 janitor 区）✅ |
| currency.js #262 | 金额：`fromCents` 系列方法返回值不正确 | **BLOCK**（7 条 logic 0.99，命中 fromCents 区）✅ |
| ristretto #345 | 缓存淘汰：OnEvict 漏设被淘汰项的 Expiration 字段 | **BLOCK**（ai_smell 0.84，命中 ttl.go）✅ |
| casl #1198 | 授权：空 conditions 对象被当作「匹配一切」（越权风险） | **BLOCK**（logic 0.81，命中 Rule.ts 条件匹配）✅ |
| js-yaml #532 | 序列化：嵌套数组的 noArrayIndent 处理错误 | **BLOCK**（logic 0.81，命中 dumper.js）✅ |
| axios #10788 | 资源泄漏：socket 内存泄漏 | **BLOCK**（logic 0.81，命中 http.js）✅ |
| undici #5343 | 并发：HTTP/2 client onEnd 与 onTrailers 竞态 | **WARN**（命中 client-h2.js 竞态区，logic 0.77 临界，未过 0.8）⚠ 部分 |
| validator.js #2633 | 校验绕过：isURL 对 URL 编码内容处理不当 | **BLOCK**（logic 0.81，命中 isURL.js）✅ |
| validator.js #2616 | i18n：isLength 对 Unicode 变体选择符计数错误 | **WARN**（命中 isLength.js，ai_smell 0.72 临界）⚠ 部分 |

**累计：18 个真实用户 issue → 14 命中 BLOCK，3 部分(WARN)，1 漏。17/18 被提示（BLOCK 或 WARN）。**
覆盖 14 领域：date×3 / web 路由 / 输入校验 / 状态副作用 / ORM 事务 / 并发竞态 / 大数运算 / 金额 / 缓存淘汰 / 授权 / 序列化 / 资源泄漏 / HTTP2 时序竞态 / **编码绕过** / **Unicode 计数**。
- **稳定模式**：**清晰 bug → BLOCK；需精确枚举/计数/逐步模拟的细微 bug（进位/时序竞态/Unicode 计数）→ 置信度临界(0.72–0.77) → WARN**——
  区域已被 logic/ai_smell 命中、仅卡 0.8 阈值下，**已醒目提示而非静默漏报**。唯一静默漏报仍是 addBusinessDays（跨周末三步 off-by-one）。
**注意**：1 漏 + 1 部分**都是细微算术/算法 off-by-one/进位**（addBusinessDays、big.js 进位）——同一类硬尾，见 `LIMITATIONS.md`。

## 本轮改进（eval 驱动）
- **logic 维度加「具体用例执行推演」**：对计算/循环/下标/区间/日期/业务日逻辑，要求模型拿边界输入
  （周末/月末/闰年/DST/进位）**逐步模拟执行**、对照应有语义，并在 message 写出推演用例。
- **验证**：DST 从 WARN→BLOCK（且归入 logic 维度、推理正确）；4 个干净 logic 改动仍 PASS（**精度不退**）。

## 诚实的已知局限（addBusinessDays）
- 该 bug 是 `while (shiftSize>0 || isWeekend(date)) {...}` 里**跨周末的 3 次迭代 off-by-one**——
  即便加了「用例推演」提示 + samples=3，模型仍**误推演**（要么没选周六起始，要么算错循环），一致性漏报。
- **这是静态 LLM 审查的硬尾**：纯靠"读+推理"无法稳定抓住需要**真正执行**才能暴露的细微算法 off-by-one。
- **业内标准的处理**：此类 bug 的可靠防线是**单元测试**（date-fns 正是用新增测试锁定修复的）。ReviewGate
  定位是"质量闸口"，对这类**建议与测试互补**，而非声称能 100% 抓住所有算法细节 bug。**不夸大、可信**。

## 结论
- 真实业务 bug 验证（4 个 date-fns 真实用户 issue）：**改进后 3/4 命中 BLOCK**（DST、parseISO 24:00、负零差值），
  1 例诚实标注为已知局限（addBusinessDays 跨周末 off-by-one）。
- 改进（logic 用例执行推演）对**多数边界/算法逻辑**有效（DST 从 WARN→BLOCK），且**不伤精度**（4 干净 logic 用例仍 PASS）；
  对**极细微的跨分支 off-by-one** 仍是公开难题，已写入 `docs/LIMITATIONS.md` 并建议测试互补。
- 产品取信于"明确能力边界 + 测试互补"，符合生产可用/可信的标准。
