# Frontier 攻坚未果：跨文件状态表征漂移（syncthing #10170）— 2026-07-02

## 目标 bug

syncthing #10170 修复「invalid 文件未计入 LocalFlags → 全局计数错」。revert 后重新引入的语义 bug：
`RawInvalid bool` 与 `LocalFlags` 位掩码构成**双事实源**，`FlagLocalRemoteInvalid` 从聚合掩码移除后，
基于掩码的消费方（计数/汇总）看不到只活在独立字段里的状态。19 文件 +70/-132，每个 hunk 各自看都自洽，
bug 是**三跳跨文件不变量**（字段↔掩码位↔计数消费方）。

## 过程（方法勘误 + A/B）

1. **假 HIT 勘误**：最初用 `lib/**` 部分 revert，仓库其他角落仍引用被删常量 →
   ai_smell 抓「重构不完整/悬空引用」high 0.99 BLOCK。这是**评测装置伪影**（真实一致性 revert 不会有），
   且与真 bug 同文件、无法按路径区分。数据集已改为**全量 revert**（`-`），此坑写入 TSV 注释。
2. **基线**（全量 revert，logic-only，600s，×2）：**2/2 零发现**（incomplete；软着陆下仍空手 = 真盲区，非截断丢失）。
3. **处理**：logic.md 加「状态表征迁移/双事实源」检查项（枚举两种表征的读者、掩码成员位增删要查聚合消费方）。
4. **结果**（同条件 ×2）：**仍 2/2 零发现**。按事先钉死的判定标准，**prompt 改动已撤销**（无效的复杂度不留）。

## 诚实结论

- 该类 bug（大 diff 上的跨文件状态漂移、无任何局部坏味道）**超出当前缺陷向静态审查的能力**，
  一条 checklist 撬不动三跳不变量推理；已尝试、已量化、不硬凹。
- **`--intent` 是该类别的正确工具**：PR 标题一句 "track invalid files in LocalFlags to fix global count"
  即含验收标准，意图评审的整体性 Agent 正是为此设计。缺陷向评审补不了意图缺失。
- 测量注意：该用例 4 次运行全部 incomplete（600s，慢 provider），run-to-run 方差大；
  它留在 dataset-recall.tsv 作为 frontier 标本，预期长期先红，不计入常规召回分母的达标预期。
