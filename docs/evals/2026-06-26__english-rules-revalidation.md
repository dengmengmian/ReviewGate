# 英文规则再验证(线上 main 二进制,默认全规则 ON)

背景:45 语言规则+system prompt 已英文化(a77b622),此前精度/召回验证在中文版上做。本轮在英文版复跑抽样。

## 精度(干净样例应 PASS)
- python clean → pass
- javascript clean → pass
- go clean → warn
- rust clean → pass

## 召回(真实 PR revert,应 BLOCK/WARN)
- JavaScript revert `140a179` → block
- Python revert `f0198e6` → block
- Go revert `9914178` → warn
- Rust revert `43e2f08` → warn

---

## 结论：英文化未让验证退化(证据已与线上对齐)
- **召回 4/4 全中**：英文规则下真实 PR revert 全部命中,JS 甚至从中文版的 WARN 升为 BLOCK。
- **精度红线守住**:4 个干净样例 0 误 BLOCK(3 PASS + go 1 例 WARN,非阻断)。
- 此前 45 语言精度 + revert 金标准是在中文规则上做的;英文化(a77b622)后本轮抽样复跑,结论一致(召回略升、精度红线不破),**证据与线上 main 产物对齐**。
- 诚实点:go 干净样例(地道 `v := v` 循环变量拷贝)得到 WARN,属轻噪音(非误 BLOCK);后续可顺手收紧 go 规则措辞,非阻断项。
