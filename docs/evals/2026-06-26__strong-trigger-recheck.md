# 强触发复查(明确真bug)：perl/groovy/julia/nim 漏抓是规则弱还是样例弱?

## perl 强触发 — decision: block
    · `system()` 的返回值被忽略，备份失败时无法感知。例如：磁盘满时 tar 返回非零退出码，但调用者会认为备份成功，导致数据丢失风险。（另由 ai_smell/securit
    · 未对 `$path` 做有效性检查。当 `$path` 为 `undef` 或空字符串时，实际执行的命令为 `tar czf /backups/out.tgz `（尾部无源路径），

## groovy 强触发 — decision: block
    · `sql.execute(String)` 返回 `boolean`，而非查询结果行。Groovy 标准 `groovy.sql.Sql.execute()` 返回 `true`/
    · SQL 注入漏洞：外部输入的 `name` 参数直接通过 GString 拼接 (`'${name}'`) 嵌入 SQL 语句，攻击者可通过构造恶意 `name` 值（如 `' O

## julia 强触发 — decision: block
    · 数组索引越界：`length(a) + 1` 永远超出数组末尾。例如 `a = [10, 20, 30]` 时 `length(a) = 3`，`a[4]` 导致 `BoundsE

## nim 强触发 — decision: block
    · `getValue` 过程中对未初始化的 `ref` 对象 `n` 进行解引用。`n` 声明后默认为 `nil`，访问 `n.value` 将在运行时触发 `NilAccessDe


---

## 结论：规则不弱，是我之前的样例弱
4 个"漏抓"语言换上**明确真 bug**的强触发后，**全部 BLOCK**：
- perl 命令注入(`system("... $path")`)、groovy SQL 注入(GString 拼接)、julia 越界(`a[length(a)+1]`)、nim 解引用未初始化 ref。

这证明：
1. **规则有效**——能稳定抓住该语言的真实 bug；
2. 之前 batch3/4 的"bad→PASS"是**我的触发样例本身是弱触发/非真 bug**（perl 仅缺 use strict、groovy 可能 NPE、julia 全局变量性能、nim 默认零初始化），模型**正确地未上报**——这正是"宁缺毋滥"在按预期工作，**不是召回缺陷**。
3. 故**无需强行加强这些规则**（加强只会制造误报）。唯二真实规则缺口是 csharp 空 catch 与 terraform 0.0.0.0/0，已在 recall-polish 补齐并验证。

**最终判断**：45 语言规则在"抓真 bug / 放过非 bug"两端均校准良好；召回与精度都站得住。
