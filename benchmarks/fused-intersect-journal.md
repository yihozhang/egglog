# FusedIntersect experiment journal

## Scope and baseline (2026-10-21)

Explore static stage generation and dynamic ordering heuristics that make the
default GJ planner use FusedIntersect for joins, plus performance fixes confined
to the fused path. Keep changes small and maintainable. All performance runs use
`--no-decomp`; validate local wins on luminal, eggcc, and hardboiled from
`~/egglog-benchmarks`. Exploration remains ongoing until the user asks to stop.

Starting commit: `4b5686be`. Existing user changes in execute.rs and
parallel_heuristics.rs add an environment switch for top-level partitioning;
they are preserved and included unchanged in both experimental binaries.
Other pre-existing untracked files are left alone. Baseline release binary:
`/tmp/fused-lab/egglog-baseline`, SHA256
`0b3e1691340000b78d6a327eeedbc13c156641be0feb8770f62243edec8d4c49`.

The local bench.py defines eleven workloads but does not pass `--no-decomp`.
`scripts/fused_bench.py` reads that list and measures prebuilt binaries with the
required flag, alternating order each round. It records per-run wall times,
exit status, output hashes, binary hashes, affinity, and experiment environment.
Use one thread (the CLI default) for initial comparisons; no other builds or
benchmarks run concurrently with measurements. Longer paired runs follow
screening. Raw intermediate artifacts live in `/tmp/fused-lab`.

## Ideas under investigation

1. Static small-cover fusion: scan a relation at least four times smaller than
   every incident relation and bind several variables together. **Design issue:**
   cached plans are built at rule declaration, often before tables contain rows;
   those sizes cannot predict runtime seminaive delta sizes. Investigate
   structural fusion instead of trusting these stale estimates.
2. Structural fusion: exploit common variable occurrence sets and functional
   dependencies to combine joins with private payload scans. Assess fixed-cover
   direction and duplicate projections carefully.
3. Runtime fused probing: order probes by current size and delay cover binding
   writes until probes succeed. Keep these separate to identify their effects.
4. Dynamic fusion: determine whether prepared index and tail metadata permit a
   small alternative to single-variable Intersect stages using actual subsets.

## First screening: strict static covers

Built a temporary planner with `EGGLOG_FUSED_GJ` modes `ratio4`, `same-occ`,
`unique-cover`, and `all`. Three timed pairs plus one warmup per workload,
one thread pinned to CPU 24; raw files `/tmp/fused-lab/static-*.json`.
All four were approximately neutral/slower. An untimed `with-plan` report on
hardboiled_conv1d_128 in `all` mode exposed why: 22,000 Intersect occurrences,
9,833 scan-only FusedIntersect occurrences, **zero fused joins**. The guard
requiring every cover variable to be planned excludes unused timestamp columns.
These runs therefore do not evaluate useful fusion and support no speedup claim.
Next version permits unused columns only when the projected planned variables
and constants cover the table's full primary key, preserving unique cover rows.

Further cache inspection corrected the initial timing assumption: the bridge
creates cached plans lazily at first execution, despite query.rs describing
declaration-time compilation. Sizes can be nonzero then, but later seminaive
delta sizes still differ. The user suggested revisiting replanning after large
statistic changes; investigate per-focus stage caching and factor-of-four size
change triggers, including planning overhead in timings.

Runtime alternatives prepared independently: singleton residual cover selection
(same logical stage, other prepared slots retained; no top-level partition),
deferred cover binding writes, probe ordering by key count, and empty-filter
early exit. Temporary switches will be removed from any retained implementation.
