# Eval: BurntSushi/ripgrep#3195 — searcher: fix regression with `--line-buffered` flag

- URL: https://github.com/BurntSushi/ripgrep/pull/3195
- base: `38d630261aded3a8e535fe85761e68af35bc462d`  head: `eb60087486a01f573cae9186fcca8973ef602dfd`
- 模型: deepseek-v4-pro；维度: logic,security,ai_smell（judge on）

## diff
```
改动文件数：3
  [Modified] CHANGELOG.md  (+7 -4, 2 hunks)
  [Modified] crates/searcher/src/line_buffer.rs  (+8 -13, 1 hunks)
  [Modified] crates/searcher/src/searcher/glue.rs  (+2 -2, 2 hunks)
```

## review
```
ReviewGate 审查中（3 个维度并行：logic, security, ai_smell）…
⚠ 以下维度未审完（超时/失败），结果可能不完整：logic。如需完整结论请放宽 --timeout 重跑。
闸口：WARN ▲ 有需关注的问题    3 文件改动 · 1 条可信发现 · 0 条已过滤

crates/searcher/src/line_buffer.rs
  • [ai_smell · low · conf 0.60] L418
    冗余的 `.as_bytes_mut()` 调用：`free_buffer()` 已返回 `&mut [u8]`，`rdr.read()` 也接收 `&mut [u8]`。`bstr::ByteSlice::as_bytes_mut()` 对 `[u8]` 是恒等映射（返回自身），此调用不产生任何类型转换或效果，属于 AI 生成代码中常见的"幻觉式冗余转换"。
    ↳ 建议：直接使用 `self.free_buffer()` 即可：`rdr.read(self.free_buffer())?;`

```
