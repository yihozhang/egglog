# FusedIntersect 优化实验总结

收尾日期：2026-10-22。实验起点：`4b5686be`。按用户要求，停止提出新方向，完成最后一个 shared-credit 对照后结束本轮探索。

## 结论

本轮保留了 **两个性能改进和一个报告正确性修复**，都已有独立提交。没有找到一个能让默认 GJ 在真实负载上普遍获益的自动融合启发式，因此不建议把 PureSize、MinCover 或本轮实验性 planner 设为默认。

- **叶子因子批量写入 action 列**：CYK 明确改善，math-microbenchmark 的 query 时间也稳定改善；更广的负载整体接近中性。
- **较大 filtered FusedIntersect 扫描前重新做动态排序**：大幅修复原生 free-join 策略的执行成本；默认 GJ 整体接近中性。这里的大幅收益是相对于原来的 PureSize/MinCover，不能解读为相对 GJ 的收益。
- **计划报告的 probe 列映射修复**：修正错误列名及可能的 panic，与执行性能无关。

最大的负面结果也很明确：选对较小的一侧仍不够。重复 join key 的 fanout、GJ 已有的因子化、选择性过滤出现的时机，以及索引构建/复用成本，都会决定融合是否划算。许多实验能改善某一条规则，甚至让一个很差的 fused 计划快数倍，但仍明显慢于 GJ。

## 测量口径与覆盖

所有性能测量使用 `--no-decomp`。主要测量为单线程、固定 CPU 24，编译和其他 engine 不与计时进程并行。候选通常先做小规模筛选，再对有希望的改动交错运行 control/candidate，并保留预热、异常值和失败记录。

“query”是报告中的规则时间之和；“wall”是进程耗时。带报告的实验还包含报告生成开销，不能直接与不带报告的 harness 相比。下文百分比均为耗时变化，负值表示更快；“倍数”明确注明比较基线。不同实验会重新构建 control，不能把跨批次绝对值串成累积加速。

| 数据集 | 实际覆盖与用途 |
| --- | --- |
| 根目录 bench.py 的 11 项 | 两个保留性能改动均完成 15 次正式配对，harness 配置 3 次预热（旧 harness 未单独保留预热记录）；其他方案按筛选结果决定是否继续。 |
| math-microbenchmark，iter=12 | 专门比较 GJ、原生策略、binary-only、因子化执行和最终 batch 改动；batch 最终确认 5 对，重排序也有复测。 |
| 外部 81 项 | `~/egglog-benchmarks` 的 Luminal 7、eggcc 32、Hardboiled 42。两个保留性能改动各完成 3 对正式测量和 1 对预热，即各 648 次进程运行，并对可疑个案复测。 |
| 合成二元 join | 覆盖方向、大小比例、fanout、重叠率、运行中大小翻转；后续补充非对称 fanout、主键及动态唯一性场景。 |
| 定向规则与诊断 | 精确选择部分规则，核对计划、每轮计数和最终大小；计数器运行只用于解释工作量，不作为性能数据。 |

早期二元 join 的一版 harness 存在 subprocess 轮询导致的 wall 量化误差，其 wall 结果已作废；这里的二元对照使用修正后的 runner。

并非每个失败候选都跑了外部 81 项。已在本地明显退化的方案停止扩大测试；报告不把“未测”写成“通过”。部分早期原生策略改变执行顺序，最终表大小相同但 match 向量略有变化，此类数据注明限制。用户指出的 eggcc `smaller` 等已知非确定性 snapshot 不作为阻断条件；其他正确性差异仍独立检查。

## 建议提交上游的 PR

### PR 1：Batch fused leaf products into action columns

已有提交：`4b0603d1abafdd774d35539fdee9b43bdb3e35db`。

对串行执行中已有的一个或两个叶子因子，直接按列填充 action batch，避免为笛卡尔积中的每行重复构造标量 bindings。保留原来的 action 批次边界、行顺序、重复变量的内层覆盖规则和取消检查；不满足条件时继续走原路径。生产代码约 152 行，另有独立回归测试；没有新的 planner 策略或调参开关。

| 确认结果 | 耗时变化 |
| --- | ---: |
| CYK，15 对 | wall **−14.25%**，15/15 改善 |
| math12，5 对 | query **−2.729%**，5/5 改善；wall **−0.774%** |
| 根目录 11 项 | 各项均值之和 −0.055%，整体中性；逐项百分比的算术平均 −1.11% |
| 外部 81 项 | 各项均值之和 +0.108%，整体中性 |

外部 Luminal 初筛有约 1% 的损失；平衡顺序的 8 对复测中，Gemma4 变为 −0.81%，Whisper 剩余 +0.38%。这些小成本应随 PR 一起披露。已有 166 项 core 测试及精确输出检查；新增测试覆盖批次边界、共享变量覆盖、取消和 fallback。收益主要来自 CYK 和 math，不能宣传为全部负载加速。

### PR 2：Re-sort large filtered fused joins before scanning

已有提交：`ee75e5f40187484ef30fd2d6472f5b1e1c09caf2`。

保留原来的 modulo-three 重排时机，并在当前剩余规模大于 32、当前阶段是带 probe 的 FusedIntersect 时额外重排。排除首尾位置，首阶段仍持有原来的顶层分区；普通 scalar Intersect 和纯 payload scan 保留既有频率。不更改评分、逻辑前缀、缓存或物化边界。执行代码净增约 12 行（含注释）。

| 对照 | query 耗时变化 |
| --- | ---: |
| PureSize，Llama / Qwen | **−92.15% / −92.19%** |
| live MinCover，Llama / Qwen | **−80.18% / −80.22%** |
| PureSize，Hard32 | −48.89% |
| live MinCover，eggcc | −5.19% |

以上均为**同一种原生策略内部**的改善，完整负载仍慢于 GJ。默认 GJ 的根目录 11 项总 wall −0.036%；外部 81 项总 wall −0.375%，按整体中性解释。math12 复测 query −0.385%，但仅 2/5 对更快，不构成稳定的新增收益。根目录 Qwen +0.67% 在 14/15 对出现，是应披露的局部代价。

生产候选通过 166 core、19 bridge、格式检查及详细计数验证。该实测基线已包含 PR 1，不能宣称在未测的独立上游基线上也有相同百分比。建议作为修复 fused 执行路径调度成本的小 PR，避免把实验性的 planner 一起带入。

### PR 3：Fix FusedIntersect reports for permuted probe columns

已有提交：`60bd0eab1980298c8347d9908d09f57578e6c99b`。

生成计划报告时，probe 的列名应来自 probe 关系自己的物理列 `scan.to_index.vars`，原代码误用了 cover 投影中的位置 `key_spec`。两者列顺序不同时可能报告错误甚至 panic。生产修复只有数行，适合独立的正确性 PR；不声称性能收益。强制融合实验已用反序物理列验证此问题。提交上游时应一并带上可独立运行的回归测试；当前该提交本身只含实现和 journal。

以上是建议的 PR 拆分，尚未创建远端 PR。提交时只选各自的代码和测试，实验 journal 可作为补充材料，无需把全部探索历史并入上游。

## 主要 planner 与策略实验

| 实验 | 关键观察 | 最终判断 |
| --- | --- | --- |
| 初版 ratio4 / same-occ / unique-cover / all | 初版保护条件让计划中没有真正的 filtered fused join；接近中性的时间不能说明融合有效。 | 无有效收益证据。 |
| 放宽静态 small-cover | Hard128 +481.7%，Llama +532.2%；融合抢先扫描，使后续过滤来不及缩小关系。 | 拒绝。 |
| 每次重规划 / 表统计变化四倍重规划 | factor-4 下 Hard128 +64%；Llama 0.142→4.355s。降低重规划频率没有解决坏计划本身。 | 拒绝。 |
| 二元 join 的固定两侧方向、运行时较小侧 | 唯一 key 的平衡例 query 57.0→35.8ms；100:1 时小侧 12.9ms、大侧 110.8ms，GJ 16.6ms。fanout=8 时 fused 约 24.5ms，GJ 17.1ms。 | 小侧有用，但随机侧不稳健；大小不是充分条件。 |
| Schema / 已缓存索引的唯一性 gate | 唯一场景保留收益，重复 key 场景可退回 GJ；真实 11 项仍中性或较慢。唯一性失效后的缓存 fallback 还有限制。 | 不推广。 |
| 单共享 key + private payload | 有些合成 join 合适；真实模型仍 +7–13%，旧 Luminal +31% 等。 | 拒绝通用策略。 |
| 相同变量关联集合、key-pair、完整 key、固定 key admission | 或机会很少，或把便宜的单值 key 绑定变成关系扫描；多种 gate 未形成广泛收益。 | 不保留。 |
| 原生 PureSize / MinCover 全局替换 | 初筛 66 次中 30 次超时，无完整负载赢家。math12 加 payload 调度后仍约 31.7–31.9s wall / 24.4–24.5s query，GJ 9.36s / 2.05s。 | 保持 GJ 默认。 |
| math12 只对 binary 使用 free join | 保留 triangle 为 GJ 仍约 +10% query。剪掉无用 payload 只恢复 3.23%，仍比 GJ 慢 7.46%。 | 拒绝。 |
| 动态 singleton stage 转换 | Hard128 +40.9%，Whisper +60.5%；逐递归构造 stage 的成本过大。 | 拒绝。 |
| 全 live cover / bridge / anchored cover | 选择性较差的 cover 会放大后续遍历；部分 anchor 规则由亚毫秒变成数百毫秒。 | 局部观察，不推广。 |
| PK-domain / fixed-key 拆分 / triangle admission | Ramp 等有局部改善，但跨规则相互影响明显；resize 的 Ramp/Ramp +16.66%、全 query +2.528%。另观测到更多小 packed nodes，未据此证明因果。 | 多轮确认后不保留。 |
| cached singleton cover 选择 | Llama/Qwen 个别规则获益；根目录 query +4.671%、wall +1.518%，Whisper 尤其退化。 | 拒绝。 |
| maximum-input refinement 评分 | 能修复一部分 Whisper 成本；根目录近中性，原生策略又在其他工作负载上显著退化。 | 拒绝全局修改。 |
| fixed-candidate relational-probe veto | 根目录整体近中性，未形成增量收益。 | 拒绝。 |
| 一个 LoopStart cover + GJ 后缀 | 能拿到原生 MinCover 的部分局部收益，仍远不及完整原生局部计划；诊断表明相关零输出执行在进入后缀前已拒绝。 | 不把收益错误归因于后缀融合。 |
| 含常量主键的 private-cover planner | 补上此前反向 lookup 无法重建常量 key 的缺口；runtime helper 有明显局部收益，但整个方案仍远慢于 GJ。 | 拒绝，详见收尾实验。 |

## 主要 runtime 实验

| 实验 | 关键观察 | 最终判断 |
| --- | --- | --- |
| 主键反向驱动 | probe 较小时用 key 查 cover；合成“cover 大、50% miss”从固定 fused 69.2ms 降至 4.8ms，GJ 10.4ms。 | 合成数据有效，真实负载不足以支持通用策略。 |
| terminal streaming / borrowed leaf | 有些路径少分配，整体收益不稳定或退化。 | 不保留。 |
| grouped terminal executor | math 有约 1% 的确认收益，但根目录总 wall +1.13% / +1.53%，Hard128、Whisper 退化。 | 放弃较大的执行器改造，收敛为 PR 1。 |
| guarded terminal singleton | math query 约再改善 1%，但 CYK、Python 等稳定损失。 | 不保留额外执行路径。 |
| single-leaf dead projection pruning | 四条目标规则约 −4.58%，只省约 2.6ms；整体 query +0.52%。 | 不保留。 |
| 按大小排序 fused probes、scalar-first、scalar-only | 若干规则稳定改善，但其他规则相反；生产候选在 CYK 确认中退化。 | 不保留。 |
| failure-driven move-to-front | 不能稳定预测后续选择性，部分原生模型退化。 | 拒绝。 |
| 延迟获取 fused probe 索引 | 原生策略内部普遍有局部收益；干净默认 GJ 候选根目录 query +0.698%，Hard128 新鲜确认 +1.172%（14/15 更慢）。 | 拒绝默认生产改动。 |
| 已发布 cover-root 预过滤 | native MinCover query −18.132%，PureSize +4.691%；同一机制在两策略的机会不同。 | 不推广。 |
| 复用已有 scalar catalog 过滤 cover | live-Min 在 Llama/Qwen/CYK 等大幅改善；直接比较 GJ 时 CYK 仍 +52.938% query，Hard128 +28.502%。 | 是局部优化证据，不是替换 GJ 的证据。 |
| Tiny tuple / singleton packed bypass / 阈值调整 | 一些开销被移走，但不能稳定减少整段查询成本；singleton bypass 在测试面板普遍损失。 | 不保留。 |
| fused leaf action batching | 避免每行 bindings 填充，保留既有因子化。 | 保留为 PR 1。 |
| 扫描较大 filtered fused 前重排 | 利用当前剩余集合，避免沿用过时阶段顺序；不增加全量重新规划。 | 保留为 PR 2。 |

## MinCover / PureSize 到底在哪些地方更快

确实存在可重复的局部赢家，不能简单说两者“都慢”。下面 PureSize 的局部百分比来自历史上的同二进制、同调度实验：GJ 和 Pure 都启用了 DEFER1/BRIDGE1。它们不是原封不动的默认策略对照，也不能与后来的严格 GJ DEFER0 基线混用。

- **PureSize 的正输出例子**：Llama/Qwen direct-exp 规则分别比 GJ 快约 39.95% / 41.73%，list-tail 约 15.54% / 15.35%；目标计数及完整模型计数一致，但整个 PureSize 查询仍约为 GJ 的 13.4 / 17.2 倍。
- **MinCover 的局部赢家**：list-tail、部分 unroll、部分 Ramp/常量运算规则。若叠加 catalog helper，某些局部或同策略比较可快 15–47%；必须区分“同策略 helper 开/关”和“相对 GJ”这两个基线。
- **不是赢家的例子**：RMS 从 PureSize 内部排序得到约 15.7% 改善后仍超过 GJ 两倍；math factoring triangle 在原生策略下约 22.3s，而 GJ 约 0.25s。

工作量诊断解释了部分 list-tail 收益：原生计划减少索引获取和构建，例如 Llama 的获取 62→31、packed 构建 30→0；但 probe 由 5,875→6,804，失败 probe 542→1,471。Whisper 的代价更明显：probe 1,768→6,605。省掉索引构建与扩大扫描域是并存的，不能只看其中一个计数。

## 最后一次尝试：constant-key 与 shared-credit

此前 constant-key helper 在旧版 Llama 中，18398 次目标 stage 调用有 18308 次实际采用主键过滤；待扫描的物理 cover 行由 882465 降为 20398（−97.69%，后续仍做原有 live/slow 检查），但 stage 调用次数没有下降。这说明功能确实激活，也说明“扫描行减少”不等于“整个计划已优于 GJ”。同一试验中 helper 相对没有 helper 的融合计划 query −32.777%，整体仍比 GJ 慢 +233.147%。

把融合 group 从逻辑前缀放回原 key 位置后，旧版 Llama 相对前缀方案 query −4.499%，主要来自 Sum；整体仍比 GJ 慢 +226.126%。Mul 中不少单项收益被另一轮调用的等量损失抵消。实测物理访问顺序也否定了“只是 dtype 更早执行”的解释。

最后的 shared-credit 实验只改变 DVO 的 refinement 更新：filtered cover 只为参与 probe 的 bind slot 累计 credit，不把私有 payload 列计入；不改变计划、评分读取、重排频率和 helper。其动机是观察到逻辑前缀 seed 会给物理上仍待执行的 group 赋予包含 payload 的 credit。只凭观察不能证明这项修改有益，因此用独立对照收尾。

最终结果：**拒绝 shared-credit，未保留代码**。173 项 core 测试在三个配置下均通过，另有 19 项 bridge 和格式检查；六份计划报告的 14,946 条记录通过核对，B/C 的完整逻辑计划相同。随后 30 次运行（每个上下文 2 组三臂预热、3 组三臂正式测量）全部成功，74,730 条记录通过输出核对，没有排除样本；独立审计重算了 5,016 组统计。

A 为严格 GJ；B 为原前缀 planner + constant-key helper；C 只在 B 上打开 shared-credit。两个上下文分别统计：

| 输入 | A query | B query | C query | C/B query | C/A query | C/B wall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 根目录 Llama | 36.429ms | 36.713ms | 37.233ms | **+1.415%** | +2.205% | +0.174% |
| 旧版 Luminal Llama | 6.245ms | 21.095ms | 21.493ms | **+1.885%** | +244.151% | +0.522% |

两个输入的 C/B query 都是 3/3 对更慢，两个执行顺序组也都更慢。旧版输入的 wall 同样 3/3 更慢；根目录 wall 较混合（1/3 更快）。这次试验没有提供继续推广的依据，因此不再增加调参、诊断或外部 81 项测量。改变 credit 的假设已完成一次直接检验，不能把之前观测到的 credit 暴露当成优化成功的证据。

更进一步的 direct-row emission 只有只读设计说明，尚未实现或计时；收到收尾要求后已取消，不作为性能结果，也不列为可提交 PR。

## 最终状态与证据入口

仅上述三个实现提交被保留；没有把实验环境开关、候选 planner 或诊断计数器留在生产实现中。用户原有的 `execute.rs` 与 `parallel_heuristics.rs` 改动保持不变。最后候选的应用、测试和反向恢复均核对原始 diff、index 和 status。

完整时间线在 [fused-intersect-journal.md](fused-intersect-journal.md)。原始输入、冻结二进制、patch、每次运行报告和审计记录位于 `/tmp/fused-lab`；这是临时目录，若需要长期复现应另行归档。主要文件或目录索引如下：

| 内容 | `/tmp/fused-lab/` 下的目录 |
| --- | --- |
| 二元 join、唯一性与主键反向驱动 | `binary-confirm`、`binary-unique-confirm`、`primary-reverse-confirm` |
| math / batch 的最终确认 | `math12-default-batch-confirm`、`root11-default-batch-confirm.json`、`external-default-batch-confirm`、`external-default-batch-followup` |
| 重排的最终确认 | `filtered-dvo-confirm`、`filtered-production-root11.json`、`filtered-production-external81` |
| PureSize 局部赢家复核 | `pure-versus-gj-local-evidence` |
| 索引构建与 probe 工作量 | `fused-work-census-diagnostics` |
| 常量 key、位置与调度诊断 | `constant-key-cover-performance-pilot`、`constant-key-placement-performance-pilot`、`constant-key-activation-diagnostics`、`constant-key-position-observation-diagnostics` |
| 最后 shared-credit 对照 | `constant-key-shared-credit-plans`、`constant-key-shared-credit-performance-pilot` |

本轮没有实现“默认 GJ 通过自动融合在所有 benchmark 上显著加速”这一最初目标。实际产出是两个有明确适用范围的性能修复、一个正确性修复，以及一组可复查的负面结果；按用户最新要求结束探索，不再开启下一轮启发式实验。
