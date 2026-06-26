# 第二轮：真正干净样例 + rules ON（期望全 PASS）

## javascript/ok2 — decision: pass

## typescript/ok2 — decision: warn
    · 函数特意检查并拒绝了 NaN，但没有拒绝 Infinity 和 -Infinity，导致无效的无穷大值被当作合法金额返回。如果业务逻辑使用该返回值进行运算（如价格、库存、金额），无穷大会产生难以预期的结果。

## c/ok2 — decision: pass

## cpp/ok2 — decision: block
    · AI 典型异味：用 static_cast<int> 将 size_t 窄化为 int 来消除编译警告，而非真正处理边界情况。当 v.size() 超过 INT_MAX 时，结果会静默截断，产生错误值甚至负数。这是\


## 第三轮：无懈可击样例（TS 用 isFinite / C++ 不窄化）
- typescript/ok3 — decision: pass
- cpp/ok3 — decision: pass
