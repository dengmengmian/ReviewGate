# 干净样例 A/B：内置规则 ON vs OFF（定位误报来源）

## javascript/ok
- ON  decision: warn    OFF decision: pass
- ON findings:
    · 函数 `eq` 返回字符串 \
- OFF findings:

## typescript/ok
- ON  decision: block    OFF decision: block
- ON findings:
    · `typeof NaN === \
- OFF findings:
    · `NaN` 被当作有效数值返回。`typeof NaN === \

## c/ok
- ON  decision: block    OFF decision: block
- ON findings:
    · 头文件包含错误：`snprintf` 声明在 `<stdio.h>` 而非 `<string.h>`。这是 AI 幻觉的典型表现——混淆了不同标准头文件中声明的函数。缺少正确的头文件可能导致隐式声明、类型不匹配或编译失败
- OFF findings:
    · 函数 `copy` 将数据写入局部缓冲区 `buf`，但从未使用该缓冲区——函数结束后 `buf` 随栈帧销毁，写入操作完全无效果。整个函数是空操作，不符合其名称所暗示的\
    · 头文件幻觉：`snprintf` 在标准 C 中声明于 `<stdio.h>`，而非 `<string.h>`。当前仅包含 `<string.h>` 会导致编译时隐式函数声明错误（C99 以上为约束违规），或产生未定义行

## cpp/ok
- ON  decision: pass    OFF decision: pass
- ON findings:
- OFF findings:

