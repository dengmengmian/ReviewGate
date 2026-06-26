# Round 5：精度压测（干净改动的误报率）

生产级 review 工具的关键指标是**低误报**。本轮用 8 个真实、正确的干净改动（重构/加固/类型化），
跨 8 语言，期望全部 PASS（无误 BLOCK）。尤其验证 Round4 放宽 judge 后精度不退。

| 用例 | 改动（均为正确改进） | 闸口 | BLOCK 项 |
|---|---|---|---|
| go-nilcheck | 加 nil 防护 | PASS | 0 |
| py-refactor | for 循环→列表推导 | PASS | 0 |
| js-validate | 参数化查询基础上加输入校验 | PASS | 0 |
| rust-question | `unwrap()`→`?` 错误传播 | PASS | 0 |
| java-extract | 抽取 `lineTotal` 方法 | PASS | 0 |
| ts-types | 加泛型/返回类型 + 越界保护 | PASS | 0 |
| cpp-constref | 传 `const&` 避免拷贝 | PASS | 0 |
| ruby-guard | 加 guard clause | PASS | 0 |

## 结论
- **8/8 PASS，0 误 BLOCK**——干净改动不被误报，即便在 Round4 放宽 judge 之后。
- 连同此前**真实干净 PR**（ripgrep#3420 / fzf#4739,4803 / got#2454 / yt-dlp#16991 全 PASS），
  累计 **13 个干净用例 0 误 BLOCK**，精度信号扎实。
- 与全面的召回验证（30+ 语言、~18 漏洞类型、5 真实 CVE 全 BLOCK）合起来，
  ReviewGate 达到生产 review 工具所需的**精度/召回平衡**：干净代码放行、真漏洞阻断。
