# 检索评测 Baseline（D2 修改前）

命令：`cargo test --manifest-path src-tauri/Cargo.toml eval_retrieval_report -- --nocapture`

> mock embedding 是字符 uni/bi-gram 哈希的确定性替身，**非真实模型**——向量通道的绝对值
> 虚高（字符袋对共享字天然高相似），只用于衡量「通道是否融合」和「词法召回是否变化」。
> 关键看 **keyword lane**：它用真实 FTS5 trigram + 当前 phrase 查询，如实反映生产词法行为。

## Baseline（HEAD，phrase 查询未修）

```
corpus: 8 docs, 10 queries {"code": 1, "en": 2, "neg": 2, "zh": 5}
      vector | R@5 1.000 | R@10 1.000 | R@20 1.000 | MRR 0.906 | nDCG@10 0.929 | n=8
     keyword | R@5 0.125 | R@10 0.125 | R@20 0.125 | MRR 0.125 | nDCG@10 0.125 | n=8
      hybrid | R@5 1.000 | R@10 1.000 | R@20 1.000 | MRR 0.906 | nDCG@10 0.929 | n=8
negatives    | top-1 fused mean 0.01639 (n=2)
```

## 读数

- **keyword R@5 = 0.125**：8 个有答案的查询里只命中 1 个（`E1021` 精确错误码）。5 个中文自然语言改写
  + 2 个英文改写**全部漏召**——这就是「整句包成 phrase 查询」的 bug，与本地复现一致。D2 目标是把这个数拉起来。
- vector / hybrid = 1.000：mock 字符袋虚高，仅证明管线连通；真实向量模型下不会这么完美。
- negatives top-1 fused mean 0.016：负样本的融合分很低，说明**存在可分离的阈值空间**（D5 用得上）；
  但当前 `fused>0` 阈值等于不过滤，负样本仍会返回结果——D5 的活。

## D2 完成后预期

- keyword lane R@5/R@10 应显著上升（中文改写查询开始命中）。
- 用同一命令对比即可量化提升；hybrid 在真实模型下也应随之更稳。

## D2 完成后实测（bigram + unicode61 + term-OR）

```
      vector | R@5 1.000 | R@10 1.000 | R@20 1.000 | MRR 0.906 | nDCG@10 0.929 | n=8
     keyword | R@5 0.875 | R@10 0.875 | R@20 0.875 | MRR 0.875 | nDCG@10 0.875 | n=8
      hybrid | R@5 1.000 | R@10 1.000 | R@20 1.000 | MRR 1.000 | nDCG@10 1.000 | n=8
negatives    | top-1 fused mean 0.03265 (n=2)
```

- **keyword R@5：0.125 → 0.875（7×）**。8 个查询里 7 个命中；唯一漏的是「多久能收到货」↔「送达」，
  属同义词无共享字，keyword 通道本就做不到——交给向量/hybrid（hybrid nDCG 已 1.000）。
- hybrid MRR/nDCG：0.906/0.929 → **1.000/1.000**。
- eval_tests 现含回归护栏：`keyword R@10 >= 0.5`，跌回 phrase 基线即 CI 报警。
- negatives 融合分仍低（0.033），可分离——留给 D5 阈值。

