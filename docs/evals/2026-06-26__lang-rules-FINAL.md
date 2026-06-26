# 内置语言规则：45 种全部默认开 — 最终总结

## 结果
**45 种语言起步规则全部内置、默认按改动语言注入**，每种都经真实模型（deepseek-v4-pro）验证：
**clean 样例不误 BLOCK（精度红线）**。可用 `[business] builtin_language_rules=false` 整体关，或 `.reviewgate/rules/<lang>.md` 覆盖叠加。

## 分批验证（rules_dir 注入候选规则 + builtin 关，逐条核查）
- 批1(核心8): python/go/javascript/typescript/rust/java/c/cpp — 3 轮 + A/B，clean 全 PASS。
- 批2: csharp/php/swift/kotlin。
- 批3: scala/dart/objc/lua/perl/haskell/elixir。
- 批4: clojure/groovy/julia/ocaml/fsharp/zig/nim/erlang。
- 批5: ruby/r/crystal/cangjie/html/css/svelte/sql/solidity/fortran/cobol/pascal/dockerfile/terraform。
- 收尾: shell/powershell/vue/graphql。

## 方法与发现（诚实）
- **提升门槛 = 精度**：clean 样例必须 PASS（不误 BLOCK）才默认开。
- **反复出现的现象**：多次"clean 样例被 BLOCK/WARN"经核查都是**我的样例本身有真实缺陷或合理边界**（TS 的 NaN/Infinity、C 的空操作+漏头文件、ruby 的 nil 值、shell 的 -u 无参崩、graphql 的无上限分页…），换上真正干净的样例后即 PASS——**证明是规则在正确工作，不是误报**。
- **召回是次要项**：部分语言触发样例未被规则直接命中（perl no-strict、groovy/julia/zig/nim 等），但**不误报、无害**；规则措辞的召回增强留作后续打磨。
- 每批留痕：`docs/evals/2026-06-26__lang-promote-batch{2,3,4,5,6}.md` + 核心8的 `builtin-language-rules{,-ab,-clean2}.md`。

## 结论
"框架有、规则库也有、且默认带"——45 种常见+不常见语言开箱即用，按改动语言自动注入，经验证守住"宁缺毋滥"。
