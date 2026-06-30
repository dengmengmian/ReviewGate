# 前端真实 PR 验证：24 PR × 5 维 + judge（全保真）

第一次把 ReviewGate 系统性地跑在**前端代码**(React / TS / Preact / 构建工具)上。此前所有 eval 都是
后端/CLI/库(Go / Rust / Python / Node)。回答一个问题:**5 维度 + judge 在真实前端合并 PR 上,
误报率如何、工程上能不能跑通 range 模式 + 大仓库 + 并发。**

> 方法学(沿用本仓库一贯做法):**真实世界已合并 PR 做 ground truth**——这些 PR 已被维护者审过、
> 合并了,本就干净,所以「干净应不刷噪音」是测**精度/特异性**。难以在真实合并 PR 里自然出现的
> 缺陷(XSS / 客户端越权 / 渲染性能),用**植入式**强触发验证维度能力,与
> [`recall-planted-bugs`](2026-06-25__recall-planted-bugs.md) 同法、明确标注(见文末「植入式补充」)。
> 全部 `deepseek-v4-pro` 真实跑通。

## 一、样本与配置

- **24 个真实合并 PR**,5 个真实前端仓库:`excalidraw`(5) · `vitejs/vite`(5) · `preactjs/preact`(5) ·
  `TanStack/query`(4) · `react-hook-form`(5)。
- **全保真**:`--dimensions security,perf,logic,style,ai_smell` + judge 反证 · 每维度 `--timeout 300` ·
  **range 模式**(`--from <PR base oid> --to <PR head oid>`,审 PR 净改动) · git worktree 隔离 · 并发 2 · 瞬时失败重试。
- PR base/head 取 GitHub `baseRefOid`/`headRefOid`,确保 diff 等于 PR 真实改动(而非与默认分支的 merge-base)。

## 二、结果汇总

| 指标 | 值 |
|---|---|
| 成功跑完 | **24 / 24** |
| 判定分布 | **0 BLOCK · 20 WARN · 4 PASS** |
| 累计发现(kept) | **0 must-fix · 4 warn** |
| 误报 | **0**(4 条逐条人工核对全部成立) |
| 未审完率 | **18 / 24 (75%)** —— 见结论②,端点慢所致 |
| 耗时 | 平均 122s,最长 345s |

完整跑完(`incomplete=false`)的 6 个 review:vite#22771、vite#22782、query#10987(PASS×4)+
query#10991、rhf#13535(WARN,各 1 条真发现)——**这 6 个是最可信子集**。

## 三、4 条真发现——逐条核查质量(0 误报)

| PR | 维度/严重度/置信 | 发现 | 核查 |
|---|---|---|---|
| react-hook-form#13539 | style/low/0.88 | `useFieldArray.ts:413` 的「根级数组错误 vs 索引子字段错误」隐晦判断逻辑与 `createFormControl.ts:561` **重复**,建议抽 `isRootArrayError` | ✅ 真实,**跨文件**定位重复处,质量高 |
| react-hook-form#13527 | style/low/0.95 | 同一值在调用链里 `skipRender` / `skipStateEmit` **命名不一致** | ✅ 真实一致性问题 |
| react-hook-form#13535 | perf/low/0.68 | `.map()` 回调内每次迭代重复 `isBoolean(disabled)` + 建对象字面量,`disabled` 为循环常量应外提 | ✅ 真实微性能,评级恰当 |
| TanStack/query#10991 | style/low/0.90 | 改的 Preact 文档指南用了和其余 36 个指南不一致的 `replace` 模式,全局批量替换会漏此文件 | ✅ 真实,精准 |

全是 style/perf · low 的可维护性/一致性 nit,**没有一个 bug 级误报**——对已被维护者审过的合并 PR,
这正是该有的表现。

## 四、结论

1. **误报率:优秀。** 24 个真实干净 PR,**0 must-fix、0 BLOCK、0 误报**。在好代码上不刷噪音、不「狼来了」。
2. **最大局限:未审完率 75%,是端点而非 ReviewGate 的问题。** 全保真 5 维度 + judge 在慢端点
   (`taotoken` 代理的 deepseek)上单维度常撞 300s 超时。**ReviewGate 行为正确**:绝不把未审完判 PASS,
   一律 WARN + 明示「does not mean no issues」。换快端点 incomplete 率会大幅下降。
3. **跑挂的不是产品,是脚手架。** mui 5 个失败 = blobless 克隆大仓库按需拉 blob 网络中断;worktree
   偶发 = 并发抢锁。均已修(改完整克隆 + worktree 重试),与 ReviewGate 无关——故用 react-hook-form 替补。

## 五、植入式补充(灵敏度,与上面的特异性互补)

同会话用一个 React+TS 组件 `Dashboard.tsx` 植入 7 类前端典型缺陷,`--no-judge` 全维度:
**6/7 命中** —— useEffect 漏依赖(logic·high)、`dangerouslySetInnerHTML` XSS(security·high)、
每次渲染重排 1 万元素无 useMemo(perf·high)、循环内 `console.log`、URL 未 `encodeURIComponent`、
`runSearch` 丢弃结果;另**额外**发现 1 万节点无虚拟化、无错误处理两个未植入的真问题。**判定 BLOCK**。

唯一弱点:`{user.role == "admin" && <button>Delete all users</button>}` 初版只判成 `==` 的 style·low。
据此**强化了 security 维度 prompt**(`security.md` 增加「客户端授权不是安全控制」+「XSS」两条),
重跑后同一处被正确提级为 **security · high · 0.90**(客户端越权,服务端必须独立鉴权),`==` 作为次要点保留。

> 复现:`worklist.tsv` / `results.tsv` / 各 PR 的 `logs/*.json` 见验证脚手架;range 命令形如
> `reviewgate review --from <baseOid> --to <headOid> --format json --timeout 300`。
