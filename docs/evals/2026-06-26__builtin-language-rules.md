# 内置语言规则验证（默认开，deepseek-v4-pro）

每语言：bad=明确命中规则的触发样例（应 BLOCK/WARN）；ok=地道写法（应 PASS，验证不误报）。维度 logic,ai_smell,security。

## python / bad
```
ReviewGate: BLOCK

```

## python / ok
```
ReviewGate: PASS
ReviewGate: PASS
No actionable issues found.
No action required.
```

## go / bad
```
ReviewGate: BLOCK

```

## go / ok
```
ReviewGate: PASS
ReviewGate: PASS
No actionable issues found.
No action required.
```

## javascript / bad
```
ReviewGate: BLOCK

```

## javascript / ok
```
ReviewGate: WARN

```

## typescript / bad
```
ReviewGate: BLOCK

```

## typescript / ok
```
ReviewGate: BLOCK

```

## rust / bad
```
ReviewGate: PASS
ReviewGate: PASS
No actionable issues found.
No action required.
```

## rust / ok
```
ReviewGate: PASS
ReviewGate: PASS
No actionable issues found.
No action required.
```

## java / bad
```
ReviewGate: BLOCK

```

## java / ok
```
ReviewGate: PASS
ReviewGate: PASS
No actionable issues found.
No action required.
```

## c / bad
```
ReviewGate: BLOCK

```

## c / ok
```
ReviewGate: BLOCK

```

## cpp / bad
```
ReviewGate: PASS
ReviewGate: PASS
No actionable issues found.
No action required.
```

## cpp / ok
```
ReviewGate: WARN

```


---

## 最终结论（3 轮 + A/B 后）

**精度（默认开的红线，最重要）：8 核心语言全部通过——真正干净的代码 rules-ON 不误 BLOCK。**
- python/go/java/javascript/c：干净样例直接 PASS。
- typescript/cpp：第一/二轮的 BLOCK/WARN 经 A/B 与逐条核查，全是**我样例里的真实缺陷或合理边界**（TS 的 NaN/Infinity 未拒、C 的空操作+漏头文件、C++ 的 `size_t→int` 窄化由 [CPP4] 正确命中），**非规则噪音**；换上无懈可击样例（TS `Number.isFinite`、C++ 不窄化）后第三轮均 PASS。
- 关键证据：JS 在我"返回字符串"的脏样例上 ON=warn/OFF=pass，但换成 `return a===b` 后 PASS——证明噪音来自样例不来自规则。

**召回：触发样例**
- 命中并 BLOCK：python(可变默认参) / go(循环变量捕获) / js(`==`) / ts(`as any` 绕过) / java(`new BigDecimal(double)`) / c(`strcpy` 越界)。
- 漏抓：rust(`pub fn` 里 `.unwrap()`) / cpp(裸 `new`) —— 召回偏软，但**不误报、无害**。

**决定：核心 8 语言全部保留默认开**，无需撤下或收紧（精度成立）。rust/cpp 触发召回偏软记为已知点，后续可在规则措辞上加强；长尾 5 语言（csharp/kotlin/php/ruby/swift）仍 opt-in，验证后再默认开。
