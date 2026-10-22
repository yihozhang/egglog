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

## Active fusion, runtime experiments, and replanning

- Dynamic singleton stage conversion: **rejected**. Three paired runs showed
  hardboiled_128 +40.9%, whisper +60.5%, paged_llama +15.8%. Constructing a
  synthetic owned stage per recursive call costs more than the tiny leader
  index it avoids. Removed from working implementation; raw data
  `/tmp/fused-lab/runtime-singleton.json` and prototype patch retained in /tmp.
- Relaxed static ratio4: **rejected**. Hardboiled_128 +481.7%, llama +532.2%;
  stopped repeated whisper runs after the regression was established. Binding
  dead timestamp variables was subsequently removed and probe key ordinals
  remapped. All surviving candidate covers require the full primary key in
  their live projection or constant constraints, to avoid duplicate bindings.
- Exposed report bug: a fused probe report used cover-relative key ordinals as
  target physical columns, sometimes panicking. Fixed it to use the target
  scan columns. The forced fused test now checks reversed physical columns.
- Per-focus replanning: always refresh and factor-four refresh both regressed.
  With factor four, hardboiled_128 +64%, llama 0.142s -> 4.355s, paged_llama
  0.160s -> 3.290s. Reducing planning frequency did not fix the main cost.
  See `/tmp/fused-lab/refresh-{1,4}.json` (qwen hit the five-second watchdog).
- Distinct-key gates, >=2 shared-variable gates, and whole-query terminal
  covers were screened with deferred binding writes, sorted probes, primary
  key probes, tiny tuple residuals, and terminal factorization enabled.
  None has demonstrated a repeatable local win. One-pass cold-start numbers
  are screening only, not confirmation; notably an apparent hardboiled gain
  in the first workload pair disappears after warmup. Raw data:
  `/tmp/fused-lab/combined-*.json`.

Untimed llama plan reports explain the regression more precisely. The baseline
has 45,836 Intersect occurrences and 15,056 scan-only FusedIntersect occurrences;
refreshed ratio-distinct has 42,538, 13,277, and 2,061 fused joins respectively.
All per-rule match counts agree. Four cublaslt beta rules dominate the extra
time: roughly 0.2ms each becomes 0.4–0.9s. Fusing all shared variables of ICons
and Add commits to a scan before other edges can prune it. Thus active fusion
and correctness alone do not establish a good plan.

Focused forced-plan tests pass at one and four workers, including reversed
tuple columns, duplicate shared-key payloads, stale cover/probe/gate rows,
slow constraints, and refreshed cached seminaive headers. Combined runtime
flags also pass those tests. Additional terminal mixed-batch tests are queued.

Next experiments: protect factorized private-key payloads, group only variables
with identical relation occurrences while leaving selective outputs as GJ
stages, and the user's explicit binary-join experiment `R(x,y) ⋈ S(y,z)`.
Compare both fixed covers and smaller-side selection using refreshed subsets,
across relation-size imbalance and join-key fanout. Ground-truth harness for
all 81 luminal/eggcc/hardboiled programs is prepared, but no candidate has yet
earned promotion to that confirmation stage.

## Binary joins: direction, fanout, and live plan choice

The requested `R(x,y) ⋈ S(y,z)` experiment uses immutable generated inputs,
retains all three output columns, and checks exact output sizes and match
counts. Across 31 cases it varies R:S from 1:100 to 100:1, join-key fanout
1/8/64, overlap 25%/100%, and an execution where the smaller side changes.
Repeated explicit runs use `--no-decomp --naive -j 1` to exercise the query
without seminaive elimination. Generator and runner are in
`/tmp/fused-lab/{generate_binary_inputs.py,binary_compare.py}`.

An initial timing harness redirected output to files while using a subprocess
timeout, causing polling-quantized wall times. Those pilot wall measurements
are invalid; internal query timings were unaffected. The runner now captures
output through pipes and writes artifacts after measuring. The confirmation
below uses the corrected runner, three measured runs and one warmup, CPU 24.
Raw artifacts: `/tmp/fused-lab/binary-confirm`.

| Case | GJ query | Fused smaller cover | Fused larger cover |
| --- | ---: | ---: | ---: |
| Equal rows, fanout 1, full overlap | 57.0 ms | 35.8 ms | 35.9–36.2 ms |
| Equal rows, fanout 8, full overlap | 17.1 ms | about 24.5 ms | about 24.5 ms |
| Equal rows, fanout 64, full overlap | 22.9 ms | about 27.0 ms | about 27.0 ms |
| R:S=100:1, fanout 1, full overlap | 16.6 ms | 12.9 ms | 110.8 ms |
| R:S=100:1, fanout 1, 25% overlap | 9.1 ms | 8.1 ms | 106.4 ms |

This confirms useful fusion for unique join keys (37% less query time in the
balanced case), but rejects random direction as a robust policy. Fanout matters:
GJ projects both payload groups once per y and factorizes their Cartesian
product. Fused execution repeats the probed payload projection for each cover
row, losing that sharing when several rows have the same y.

A separate scheduling issue initially obscured the binary comparison. When a
fixed fused cover is larger than the other side, DVO can select that side's
private payload scan first and scan the entire cover once per payload row.
The experimental `EGGLOG_FUSED_DEFER_PAYLOADS` policy delays unconstrained
payload-only scans until incident fused filters finish. It preserves constrained
scans, materialization barriers, and factorized leaf scans. Focused tests cover
both initial stage orders. These measurements include this policy.

`binary-small` now caches the two possible orientations and selects using exact
current filtered subsets. There is no full replan on repeated orientation hits.
It follows the changing smaller side in the swap fixture, but fanout-8 fusion
still loses to GJ. This is a cost-model issue, not merely refresh frequency.
Local 11-workload screening (three paired runs plus one warmup) is approximately
neutral/slower: hardboiled_128 +1.2%, luminal +2.7%, whisper +1.2%; all other
changes between -0.5% and +2.6%. Enabling deferred bindings, probe ordering,
primary-key tuple lookup, tiny tuple residuals, and terminal factorization
changes this little. Raw data: `binary-cache-local.json`, `binary-all-local.json`.
No workload speedup has been established, so these prototypes are not promoted.

Next gate: shared keys/constants must determine the smaller cover's row, either
from its schema or an already-current cached scalar index proving uniqueness.
Checking cached indexes never creates or refreshes them. The runtime cache must
recheck this proof before reuse and avoid caching negative decisions so a later
GJ-built index can enable fusion. Experimental fallback currently retains the
original cached stages if uniqueness later disappears: semantically valid, but
not a guaranteed performance fallback to GJ. A retained version needs an
explicit unfused fallback. Also test borrowing an immediately consumed terminal
leaf's rows, avoiding an Arc without changing action batching.
