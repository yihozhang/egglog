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

## Uniqueness gate and private-payload extension

Cached-index uniqueness preserves the binary benefit and avoids repeated-key
losses. Corrected runner, three timed runs plus one warmup, one thread, CPU 24:
`/tmp/fused-lab/binary-unique-confirm` compares modes in the same frozen binary.
Equal-size fanout 1: GJ 63.3 ms, ungated fused 39.1 ms, gated 40.5 ms. Fanout 8:
GJ 17.2 ms, ungated 23.9 ms, gated 17.1 ms. With 100:1 imbalance, gated query time
is 13.7–13.8 ms versus GJ 17.9–18.0 ms. On the fanout 8 smaller-side swap, gated
15.9 ms matches GJ 16.0 ms; ungated takes 26.9 ms. These runs also enable borrowed
terminal leaves; that change alone has no demonstrated local workload benefit.

Four transition fixtures validate uniqueness becoming duplication and returning,
for either smaller side, both sides, and a mutation with unchanged table size.
A fifth warms the R join-key index before compiling the cached rule. Every
iteration's match count and cumulative output-size snapshot matches its oracle
for GJ, fixed/smaller fusion, and gated fusion. Artifacts:
`binary-transitions-check`, `binary-warm-transition-check`. These are correctness
runs, not performance confirmation. Stale original fused fallback remains the
performance caveat described above.

The schema-only and cached-index binary gates remain roughly neutral/slower on
the 11 local workloads; no basis for promoting them as a broad improvement.
Raw `binary-{unique,cached-unique}-local.json`. Full core tests pass: 171/171,
including primary tuple probes, stale rows, cancellation, materialization, and
parallel fused fixtures.

Two broader modes fuse exactly one shared variable with schema-determined
private payloads, preserving all other GJ variables. `private-unique` always
chooses an eligible cover; `private-small-unique` also requires its initial
size <= every incident relation. With factor 4 refresh, they regress, including
python +61%/+11%, llama +29%/+20%, luminal +20%/+15%. Without refresh, small
private fusion is mostly neutral but luminal +34%. Existing llama reports have
910 structurally eligible covers across 112 rules, whose total measured time is
only 10.8 ms; this particular shape has limited upside there. Static selection
still fails when an earlier selective GJ stage shrinks a probe but not the cover.

Streaming an eligible serial terminal fused scan directly into action batches
avoids a row buffer, while retaining the factorized/parallel alternatives.
Standalone `off-stream-local.json` is neutral/slower; no promotion. Prototype
and tests remain available for evaluation, not accepted performance changes.
Next experiment reverses a fused scan at execution time when a much smaller
probe's distinct keys can look up cover rows by their unique primary key. It
requires no stage recompilation, and preserves current cover-subset membership.

## Execution-time driver reversal

A scalar-primary-key prototype enumerates a much smaller probe's distinct keys
and looks up the cover row, checking membership in the current cover subset.
It preserves sparse/dense headers, stale-row rejection, probe fanout, and tail
refinement without rebuilding stages. A later version supports permuted complete
primary keys and validates extra leader values. Tiny exact tuple probes gained
bounded duplicate-aware enumeration so they can also drive this branch.
Focused differential coverage plus the full core suite pass (174 tests).

The six primary-key binary fixtures in `primary_binary_inputs` isolate this
from replanning (refresh factor 0). Three measured runs and one warmup:

| Case | GJ query | Fixed fused | Runtime reversal |
| --- | ---: | ---: | ---: |
| Equal, unique keys | 55.5 ms | 36.7 ms | 37.3 ms |
| R smaller, S fanout 8 | 26.9 ms | 22.6 ms | 23.0 ms |
| R much larger, 50% misses | 10.4 ms | 69.2 ms | 4.8 ms |
| R larger, S fanout 8 | 15.0 ms | 43.7 ms | 11.6 ms |
| Selective prefix, fanout 64 | 1.9 ms | 116.8 ms | 1.8 ms |
| R size swap, fanout 8 | 15.2 ms | 27.8 ms | 11.5 ms |

All exact fixture counts pass. Raw: `primary-reverse-confirm`. Allowing DVO to
estimate this reversed driver's residual size makes little difference here.
On the 11 real workloads it reduces the private-cover python regression from
+740% to +2%, but private-cover fusion still regresses luminal +31% and larger
model workloads +7–13%. Same-incidence key-pair modes remain neutral/slower.
These are not accepted broad performance changes.

Untimed reports show a concrete eligibility gap: `nth_from_end(list, n)` has a
constant n column omitted from the bound projection. Schema uniqueness accepts
it, but the reverse prototype cannot reconstruct constant primary-key columns,
so it still scans the whole cover. This accounts for two large zero-match
regressions in luminal. All per-rule per-iteration match totals in these reports
agree. Stage-info currently emits null stage statistics, so it supplies no
physical visit/index/probe counters; conclusions beyond plan shape are limited.

## New priority: math-microbenchmark, 12 iterations

At the user's request, focus on math before requiring broad workload gains.
`/tmp/fused-lab/math12.egg` differs from `tests/math-microbenchmark.egg` only by
`(run 11)` -> `(run 12)`. It retains printed sizes and stats. The dedicated
`math_compare.py` clears inherited experimental variables, freezes binaries,
interleaves treatments, and records wall time, search/action, merge, rebuild,
per-rule counts, per-iteration counts, and final table sizes. All runs retain
`--no-decomp -j 1`, CPU 24; seminaive evaluation stays enabled.

Initial screen, three timed runs plus one warmup (`math12-initial`): baseline
9.2605 s wall / 2.0259 s summed rule time; same-binary GJ 9.2840 / 2.0443 s;
smaller-side cached binary fusion 9.6543 / 2.1958 s. Its query penalty is mainly
Add associativity (+12.2% per match), distribution (+5.1%), and Mul associativity
(+17.2%) versus same-binary GJ. Baseline rebuild is about 5.0 s and merge 2.1 s;
iteration 12 accounts for nearly all of the work. Final sizes agree exactly
(Add 11,396,002; Mul 4,188,378). Binary fusion produces stable but slightly
different per-rule match counts beginning at iteration 7; this is an
order-sensitive result, not a strict match-count oracle pass. The count changes
are too small to explain the timing penalty.

PureSize and MinCover free-join strategies, with and without payload scheduling,
all exceeded the initial 10-second watchdog. That cutoff is too close to the
baseline to rank them. A separate 60-second diagnostic (`math12-freejoin-diagnostic`)
confirms both unscheduled variants time out. With payload scheduling, PureSize
takes 31.926 s wall / 24.546 s query and MinCover 31.736 / 24.364 s, versus
GJ 9.361 / 2.053 s. Almost all query regression is the factoring triangle:
0.252 s under GJ versus 22.514 / 22.325 s, with similar match counts. Final
sizes agree; scheduled strategies agree with each other on matches but differ
slightly from GJ. Existing free join is applied only to default GJ rules by
the temporary strategy switch; explicit rebuild strategies are intact.
Math has 4 one-atom rules (already MinCover), 18 binary rules, and 2 triangles.
Eight binary constructor-output joins are the main targets; their shared key
usually is not a primary key, so the reversal prototype does not help them.
Investigate repeated payload projection and grouped terminal execution while
preserving GJ's factorization, rather than applying whole-cover fusion blindly.

The runtime screen (`math12-runtime`, three timed runs plus one warmup) has no
winner. Same-binary GJ averages 9.3204 s wall / 2.0450 s query; cached binary
fusion 9.5442 / 2.1995 s; streaming terminal payloads 9.8072 / 2.2531 s (one
2.541 s query outlier); combined runtime flags 9.6412 / 2.1061 s. The latter
reduces the query penalty to 3.0% but still regresses wall time 3.4%. Stream's
median query time also regresses 3.3%, so the outlier does not hide a win.
Candidate variants agree on exact per-rule/per-iteration counts; all final
sizes agree with GJ, with the previously noted order-sensitive count changes.
Combined flags leave Add associativity +7.4%, distribution +1.5%, and Mul
associativity +8.8% versus GJ. No performance implementation is promoted.

Next screens isolate PureSize/MinCover to binary queries (keep triangles GJ),
and fuse execution of the two terminal payload groups after a scalar binary
intersection. The grouped prototype preserves distinct-key grouping, leaf
constraints, factorization, and the ordinary per-group dynamic tail order.

The grouped-tail screen (`math12-tail-screen`, three timed runs plus one
warmup) gives query improvements of 2.77%, 1.50%, and 2.27% in paired runs;
wall changes are -1.09%, -0.33%, and +0.42%. Query medians are 2.0493 s for
GJ and 2.0037 s grouped. Factoring saves 22.7 ms (249.1 -> 226.4 ms), about
half of the query gain; Add associativity saves 9.9 ms, distribution 4.4 ms,
and Mul associativity 2.8 ms. All recorded per-iteration counts, all 352
individual seminaive report entries, and all final sizes match GJ exactly.
Full core tests pass with the prototype enabled (175 tests), including
filtered/stale groups, singleton/empty groups, prior factors, physical tail
order changes, merge/gallop/hash intersections, and one/four workers.

An untimed trace (`math12-tail-trace`) sees 359,512 eligible invocations,
128,490 with no matching key, 636,268 direct keys, and 14,308,141 raw pairs.
The 4,491 recorded reorder events count DVO calls, not necessarily swaps.
It avoids one buffered frame per key and two nonzero frame-buffer allocations
per eligible invocation; projected multirow factors still allocate buffers
and Arcs. Trace output itself adds substantial overhead and is not a timing.

Binary-only PureSize/MinCover both regress query time about 10%, despite
keeping triangles on GJ. Their match vectors agree with each other across
every run; total matches differ from GJ by +0.0913%. PureSize's rebuild time
varies by about 0.5 s at iteration 12 despite identical match counts; its
apparent wall improvement in some runs is not evidence of a query gain.
A separate pass tests pruning unused private non-key payloads/timestamps;
its full core suite passes (176 tests). Two timed runs plus one warmup
(`math12-prune-screen`) give GJ 9.2800 s wall / 2.0477 s query, MinCover
binary 9.7776 / 2.2740 s, pruned 9.6740 / 2.2006 s, and pruned plus runtime
flags 9.6296 / 2.1816 s. Pruning recovers 3.23% of query time relative to
MinCover binary but still regresses 7.46% versus GJ. Add associativity and
distribution account for 88% of the recovery. Candidate count vectors agree
with each other, with the same order-sensitive differences from GJ; final
sizes agree. This planner experiment is also rejected.

Rejected prototype code has been removed from the working tree, with a full
temporary patch and frozen binaries retained under `/tmp/fused-lab`. The
remaining grouped-tail candidate is isolated for confirmation against the
original baseline. A sampled CPU profile (`math12-gj.perf`, 927 samples) has
its largest exclusive costs in table merge, mutable table lookup, and memory
movement; run_plan itself is about 1% exclusive. Release unwinding does not
provide reliable inclusive stacks, so this is not a strict bound on join
cost or a reason to attribute all query time to search.

The isolated grouped candidate passes all 166 remaining core tests. A second
small experiment replaces the two terminal factors' per-key Arc allocations
and variable collections with borrowed views through the same product loop;
it also passes all 166 tests. Frozen binaries `egglog-grouped-clean` and
`egglog-grouped-borrowed` will be compared with the original baseline before
promoting either implementation.

Five interleaved timed runs plus warmup against the original frozen baseline
(`math12-clean-confirm`) confirm much smaller gains: paired query medians
-0.744% grouped (4/5 faster) and -1.427% borrowed (5/5 faster). Query means
are 2.03975 / 2.02328 / 2.01304 s. Paired wall medians are only -0.396% and
-0.196%, with phase-time outliers. Every final size and all report-entry
match counts agree across all 15 timed runs. The 11-workload confirmation
(`root11-clean-confirm`, five timed plus warmup) regresses the geomean by
0.42% grouped / 0.53% borrowed, and summed means by 1.13% / 1.53%.
Hardboiled-128 regresses 2.52% / 3.21%, whisper 3.11% / 4.40%; cykjson gains
2.16% / 2.91%. All 165 timed runs succeed with equal per-workload stdout
hashes. Neither implementation qualifies for promotion as-is.

A temporary serial RHS timer (`math12-rhs-diagnostic`, removed from source
after freezing its binary) separates full action batches from the final
partial flush. Full RHS is 1.56197 s of 2.01614 s summed rule time, or 77.47%.
The remaining 0.45417 s includes joins, projection, batching, state creation,
clearing, counters, and diagnostic overhead; it is 4.89% of 9.28593 s wall.
Final partial RHS is 0.00112 s and is outside the per-rule query timers but
inside ruleset search/apply time. Counts and sizes equal the baseline.
Largest non-RHS residuals: factoring 197.63 ms, Add associativity 116.22 ms,
distribution 68.08 ms, and Mul associativity 22.74 ms. This makes factoring
the largest remaining search target despite its much smaller output count.

Next temporary experiments: fill existing action columns directly from a
grouped Cartesian product at the same 128-row batch boundaries, and cache
terminal payload projection by scalar key within a filtered binary fused
invocation. The latter keeps cover-row order and bounds retained storage to
4096 keys / 262144 logical cells, bypassing new cache entries beyond the cap.

User clarification: eggcc snapshot differences confined to `smaller` and its
related tables are known nondeterminism and are not a blocker for performance
experiments. Record those differences separately from unexpected changes.

The six-treatment cache/batch screen (`math12-cache-batch-screen`, two timed
runs plus warmup) passes exact counts within its two established ordering
families. Query / wall means: original baseline 2.03493 / 9.28876 s; grouped
1.99188 / 9.22916 s; grouped+batch 1.94474 / 9.24765 s; pruned binary fused
2.17314 / 9.61338 s; fused+cache 2.07964 / 9.63368 s; fused+cache+batch
2.14554 / 9.64121 s. Batch improves query time over grouped by 2.10% and
2.63%, with inconclusive wall changes. Cache improves query time over fused
by 4.78% and 3.82%, but still trails GJ. No promotion.

Small-cache tracing sees 359,383 hits and 744,977 projections; it avoids
39.28% of payload-row projection work, but 639,027 projections bypass full
budgets. A larger diagnostic (65536 keys / 4194304 cells) eliminates every
bypass: 856,804 hits, 247,556 projections, 1,717,557 projected rows, or
87.66% of projection work avoided. Peak retention is 31862 keys / 1065780
cells. All invocation/report counts and final sizes agree. However, 133 of
184 invocations have no cache hits, so caching has a real cold-key cost.
These diagnostic runs do not establish a speedup.

Binary inspection identifies another cost of grouped execution: the 12
run_plan specializations grow 17.2% in text size and common stack frames
grow 576–608 bytes. This is a plausible contributor to the broad regressions,
not proof of causation. An optional helper extraction is saved outside the
working tree. A smaller alternative batches the existing FusedIntersect /
FusedIntersectMat BindingSet directly in the serial action buffer, leaving
run_plan and the planner unchanged. It replaces the larger pending prototypes
in the working tree and passes all 166 core tests; measurements are next.

The isolated batching screen (`math12-minimal-batch-cache-limits`, two timed
runs plus warmup) gives original / same-binary disabled / enabled query means
2.02778 / 2.03092 / 1.97635 s, and wall means 9.20968 / 9.27883 / 9.20095 s.
Enabled versus disabled improves query time 2.69% (paired 2.44% / 2.93%);
wall versus original is neutral (-0.10%). Every final size, per-rule and
per-iteration count, and all 352 seminaive report-entry counts agree exactly.
Add associativity and distribution account for 85% of the query gain.
Larger fused payload caching still does not beat original GJ overall: query
1.99627 s (-1.55%) but wall about 9.407 s (+2.14%). Cache variants remain
within the previously documented order-sensitive free-join count family.

The isolated batching root-11 confirmation (`root11-minimal-batch-confirm`,
five timed runs plus warmup) has -1.36% geomean but +0.12% summed mean time.
Cykjson improves 13.66%, with all five pairs between 13.05% and 14.21% faster.
Whisper regresses 1.38%, with all five pairs 0.71%–2.12% slower. Most other
cases are within 0.7%; Hardboiled-32's apparent 2.07% mean gain is driven by
a baseline outlier (paired median only -0.18%). All 110 timed runs succeed
and every per-workload stdout hash agrees. Next screen: a minimum product
size of 0/8/32/128 for batching, to avoid tiny-factor overhead. Also running
a first external 81-workload screen; one repetition is only diagnostic.

The factoring residual trace (`math12-grouped-residual-trace`) identifies a
more selective runtime opportunity. Of 359323 factoring-tail invocations,
226588 (63.06%) have a singleton source on at least one side. Reduced Add
has median one row versus 417 rows for reduced Mul. Most sources and probers
already use packed caching, so avoiding index construction alone is unlikely
to explain a large gain. A small terminal singleton fusion prototype will
project the singleton once, probe only the other side, and retain existing
factors and persistent dynamic tail ordering.

The first external ground-truth screen (`external-minimal-batch-screen`,
81 cases: Luminal 7 / eggcc 32 / Hardboiled 42; one timed pair, no warmup,
CPU24, one thread, --no-decomp) completes all 162 runs with no final-size
warnings. Geometric-mean candidate time regresses about 0.39%; mean of
per-case percentage changes is +0.390%. Most differences are small; there
is no broad improvement. This screen is baseline-first for every case and
only one repetition, so it is diagnostic rather than a significance claim.
Full commands, binary hashes, raw outputs, and explicit inherited fused
environment are retained with the results under /tmp/fused-lab.

The threshold screen (`minimal-batch-threshold-screen`, five timed root
pairs plus warmup and two timed math runs plus warmup) does not establish a
benefit from a minimum product size. Thresholds 0/8/32/128 all improve
cykjson about 14.5%–15.1%, keep Hardboiled-128 within 0.3%, and keep whisper
within 0.4% by mean. The previous whisper regression does not reproduce even
with threshold zero in this rebuilt candidate; thresholding alone therefore
does not explain it. Math query means are 2.05000 s baseline and
1.97676 / 1.97502 / 1.97542 / 1.97922 s for the four thresholds (3.45%–3.66%
faster), with wall 9.26415 s baseline versus 9.16270–9.21657 s. Every output
hash and recorded math count agrees. Retain no minimum-size policy without
stronger evidence.

Unlike the rejected grouped executor, isolated batching leaves run_plan and
expand_binding_sets code and stack sizes essentially unchanged: run_plan text
372624 -> 372540 bytes; one frame shrinks 16 bytes and eleven are unchanged.
The bulk helper is inlined into a single serial factorized-action dispatcher,
which reserves 424 bytes even on fallback. A tiny helper split will test
whether this avoidable fallback cost accounts for broad small regressions.
The terminal singleton fusion prototype now passes all 167 core tests,
including direct ordered oracles for either side, filters, stale rows, DVO
changes on rejected groups, prior factors, and cancellation. Its performance
screen compares original, same-binary control, batching, singleton, and both.

The singleton screen (`math12-singleton-tail-screen`, two timed runs plus
warmup) preserves every final size and all 352 report-entry counts in all
15 executions. Query means: original 2.03497 s, same-binary control 2.03974,
batching 1.98556, singleton 2.02042, combined 1.96507. Singleton improves
query time 0.87% / 1.03% versus control, with factoring 250.64 -> 218.22 ms
(-12.93%); Add associativity/distribution losses offset part of that saving.
Combined improves query time 0.49% / 1.57% versus batching. Its apparent large
wall gain includes a 293 ms rebuild outlier, so it is not attributed to joins.
The helper avoids the rejected grouped executor's run_plan stack growth, but
its roughly 1 KB frame was entered before cheap eligibility rejection. A
five-line caller guard now checks the terminal shape and singleton sizes.

The thin batching dispatcher works as intended in machine code: wrapper text
3371 -> 414 bytes, stack reservation 424 -> 40 bytes, with the unchanged bulk
loop separately outlined. It passes all 166 core tests. Root11 confirmation
(`root11-thin-batch-confirm`, five timed plus warmup) is neutral by summed
means (-0.02%) and improves cykjson 14.53%; luminal-llama regresses 3.20% in
that session. In the subsequent three-way confirmation this regression does
not reproduce: batch luminal +0.55% with mixed pairs, summed time -0.26%,
cykjson -15.05% in every pair. Avoid attributing small cross-session changes
to outlining alone.

The three-way screen (`root11-singleton-preguard`, five timed plus warmup)
compares original, isolated thin batching, and guarded singleton plus batching.
All 165 timed runs succeed with identical per-workload output hashes. Adding
singleton fusion loses 2.56% on cykjson and 0.625% on Python-array versus
batching in every pair; whisper loses 0.93% (one larger outlier). Combined
summed time versus original is only -0.057%. There is no broad benefit from
retaining the extra singleton executor, despite the math factoring improvement.
A three-run math confirmation remains in progress; the smaller batching
change is the better production candidate.

Default multi-atom GJ already removes private unused payload columns, so
repeating that planner optimization cannot help its hot binary math rules.
The bridge forces single-atom queries to MinCover, however, retaining dead
columns in four math rules. A separate small single-atom/single-stage pure
fused-scan pass removes only unused projection columns while retaining row
multiplicity, constraints, and one column when all are dead. Expected widths:
Add/Mul commutativity 4->3, Sub a b 4->3, Sub a a->0 3->1. This is the next
small planner experiment; it is separate from the rejected singleton runtime.

Guarded singleton math confirmation (`math12-singleton-preguard-confirm`,
three timed runs plus warmup) again preserves all 352 entries and final sizes
in all 16 executions. Against its same-binary batch-on control, query improves
0.95% / 1.43% / 1.38%, with factoring 245.60 -> 220.77 ms (-10.11%)
accounting for essentially all savings. Wall paired median is only -0.097%.
Together with the repeated root11 costs, this is insufficient to keep the
extra executor. Its code and tests are archived under /tmp/fused-lab.

Single-leaf pruning (`math12-prune-single-leaf-screen`, two timed plus warmup)
passes all 167 core tests and exact counts in all 12 measurements. The four
affected rules improve 57.02 -> 54.40 ms (-4.58%), saving 2.54 / 2.69 ms per
pair. Overall query instead changes 2.04607 -> 2.05663 s (+0.52%), with
losses in unchanged binary rules. Apparent wall gain includes a 348 ms rebuild
outlier and is not a pruning speedup. The small targeted benefit does not
establish an overall gain; this planner experiment is removed from main.

The production candidate now contains only default serial factor batching,
without experimental environment flags or product thresholds. Its stronger
test includes an RHS variable present in both factors, confirming inner-value
precedence across 128-row boundaries. All 166 core tests pass. The generated
dispatcher shrinks again to 151 bytes and only an 8-byte alignment push, versus
414 bytes / 40 local bytes / six saved registers with flags. Join-loop and
ordinary expansion layouts remain unchanged. Final confirmation uses the
root harness's full 15 timed repetitions plus three warmups, followed by math
and an interleaved three-timed/one-warmup external 81-case confirmation.

## Clean default candidate confirmation (2026-10-22)

`root11-default-batch-confirm`: 15 timed repetitions plus three warmups,
original baseline versus the clean default implementation, CPU24 and one
thread with --no-decomp. All 330 timed runs succeed with exact per-workload
stdout hashes. Cykjson improves 14.25% in all 15 pairs (range 12.87%–17.05%).
The arithmetic mean of workload changes is -1.11%, passing bench.py's policy;
summed mean wall time is -0.055%, effectively neutral. Excluding cykjson gives
+0.177% summed time. Other means are within 0.3% except eggcc-extraction
+1.05%, whose paired median is +0.27% and one +18.15 ms pair contributes
65% of its net loss. This outlier is retained in the official mean.

`math12-default-batch-confirm`: five timed pairs plus one warmup. All 12 runs
preserve every final size and every one of 352 seminaive report-entry counts.
Query mean 2.044947 -> 1.989148 s (-2.729%); all five pairs improve
2.23%–3.02%, paired median -2.86%. Wall mean 9.278480 -> 9.206623 s
(-0.774%), with all five pairs improving 0.29%–1.21%. Add associativity and
distribution account for 90% of query savings. Merge/rebuild also improve
slightly, so approximately 78% of wall reduction is attributable to query
time. The unchanged factoring triangle is slightly slower (+2.47 ms), much
less than the repeatable gains elsewhere.

`external-default-batch-confirm`: all 81 cases, three timed repetitions plus
one warmup, alternating order. All 648 runs succeed, all-phase sizes match,
and independent checks find identical printed per-rule match-count multisets
for every case. Summed mean wall time 80.78886 -> 80.87593 s (+0.108%);
geometric mean of mean ratios +0.138%, arithmetic mean +0.139%. Suite summed
changes: Luminal +0.971%, eggcc -0.088%, Hardboiled -0.034%. This is broadly
neutral, not an across-the-board win. Candidate-first pairs have geomean
+0.014%, versus +0.200% for baseline-first, illustrating subpercent order
sensitivity. Gemma4_moe (+1.515%) and whisper (+1.237%) need balanced
follow-up; raytrace improves 0.273%, and prior conv1d192 slowdown does not
repeat (-0.458%, all three pairs faster). No snapshot warning arises in this
confirmation. The known smaller-table nondeterminism remains non-blocking.

The two uncertain Luminal cases have identical rule match counts across all
eight runs. Gemma4 wall pairs +1.46%/-0.47%/+3.56% track query variation in
merge-FE-FE-Add/Mul. Whisper input and header are byte-identical to the root
workload, but external wall pairs are +1.20%/+2.20%/+0.31%; search accounts
for most of the mean loss. An eight-pair follow-up balances four pairs in
each orientation before the implementation is committed. Formatting and
independent production-patch scope review pass; existing user edits are
explicitly excluded from the prepared production patch.

The balanced follow-up (`external-default-batch-followup`, eight timed pairs
plus one warmup, four timed pairs in each orientation) completes all 36 runs
with no size warnings. Gemma4_moe reverses its initial regression: wall means
9.6337 -> 9.5557 s (-0.81%). Whisper's loss shrinks to 0.38%
(0.4608 -> 0.4625 s). The larger initial Luminal losses are therefore not
consistent across sessions. Retain the modest default batching change for
its repeated cykjson and math gains; describe the broader suites as neutral,
not universally faster. The rejected runtime/planner prototypes stay archived
outside the working tree. Further exploration will use actual factor-count
telemetry before considering extension to three or more factors.

## Factor census and local strategy comparison (2026-10-22)

An instrumented, serial-only action-boundary census compares production with
the same code plus counters on gemma4_moe, whisper, and conv1d_128. All six
executions succeed; final sizes and printed per-rule match counts agree.
The diagnostic passes all 166 core tests and is frozen separately; its patch
is removed from the working tree. Every instrumented timing is explicitly
excluded from performance evidence. Raw records and aggregates are under
`/tmp/fused-lab/factor-census-external`.

Neither Luminal case presents any product with three or more non-singleton
factors. Gemma4_moe emits 84,044 rows (25,453 scalar, 58,401 one-factor,
190 two-factor); whisper emits 19,689 (5,540 / 14,019 / 130). The expensive
gemma merge-FE-FE rules emit exactly 2,470 Mul and 190 Add scalar bindings,
all successful. Their cost is in join search, with no factor-product batching
opportunity. Conv1d_128 has only 118 three-or-more-factor calls producing
2,268 rows out of 126,237 emitted (1.80%); these belong to two Broadcast
rewrite directions. This does not justify extending the batcher yet.

The user suggests inspecting local MinCover/PureSize wins, even when whole
workloads lose. Prior native-strategy measurements cover math; the earlier
root11 and synthetic binary studies mostly test custom fusion heuristics,
not native MinCover/PureSize. Prepare a clean same-binary native-strategy
comparison on the current production base, retaining optional payload
scheduling as an independent treatment. Collect per-rule times and counts
alongside workload times, then retest promising rule families selectively.

The clean native-strategy root11 pilot (`strategy-root11-pilot`) uses one
frozen executable, six treatments (GJ/PureSize/MinCover, each with and without
payload scheduling), one sample, CPU24, one worker, and --no-decomp. Identical
time-only reports are requested for every treatment; serialization increases
wall time relative to the ordinary root harness. Adaptive watchdogs bound the
screen. Of 66 attempts, 36 complete and 30 time out. Both GJ controls finish
all 11 cases. Raw PureSize/MinCover each finish 3/11; scheduled variants each
finish 4/11. No native whole-workload winner appears. All completed final
sizes agree; eggcc-extraction native strategies change rule/iteration counts.
The completed-case query regressions remain large with scheduling: about
14.4x on CYK, 11.9x on old luminal-llama, 17x on Python-array, and 1.5x on
eggcc-extraction. Timeout cases have no final report and cannot be ranked
beyond their watchdog observations.

The largest apparent local gain is a three-atom Luminal constant fold,
`MMul(MNum(a), MNum(b))`: about 55 microseconds in the initial broad sample.
An exact-selector follow-up (`native-mmul-followup`) changes only that rule,
with 12 timed repetitions plus two warmups and independent scheduling
controls. Selection tracing confirms exactly one three-atom rule changes.
Across all 72 timed reports, every one of 1,155 whole-workload seminaive
entries and all 50 target entries agree, as do final sizes; the target has
62 matches. Against GJ with scheduling, target means are 145.758 us versus
131.846 us for PureSize (-9.54%) and 133.237 us for MinCover (-8.59%); all
12 paired target timings improve. Whole-query means instead change +0.33%
and +0.20%, with no process-level win. Without scheduling, the same native
rule takes about 911 us versus GJ's 146 us. Its plan is an acyclic star:
an MMul cover probes two MNum output columns, then scans numeric payloads.
The inverse MNum lookups are not proven unique by the physical primary key.
This is a small local strategy win, not a production selection policy.

Temporary per-plan begin/done tracing (`native-rule-trace`) exposes the
previously incomplete cases; its durations are diagnostic only. Hard32
scheduled native runs finish under a longer watchdog, with small order-
sensitive count changes and a large ConvolutionShuffle+ search regression.
Whisper native runs both stop in `cublaslt row-order 2d scaled c beta`.
Completed aligned prefixes already show multi-second regressions in two
batch-collapse rules versus millisecond GJ times. The strongest aligned
Hard32 hint is a zero-output WMMATileB/RowMajor rule, saving about 150 us
in diagnostic completed calls; exact-selector timing follow-up is separate.

Code inspection explains the identical native strategies in these bridge
queries. Each regular atom receives a fresh private timestamp variable;
BucketQueue includes every atom variable, even private unused non-key ones.
No other atom can cover that timestamp, so every atom becomes an eligible
cover and MinCover sorts the same complete set as PureSize. The earlier
dead-projection pruning ran after cover selection and could not change this.
A new constrained experiment will remove only dead non-key variables from
the cover-selection universe, retaining shared/RHS variables and all key
variables. Selected-cover projections remain unchanged; atoms with no
retained witness or unproven key columns must remain mandatory. This is
expected to affect larger patterns with redundant covers, not hot math
rules whose live private variables already make their atoms mandatory.

`native-binary-confirm` directly compares native GJ/PureSize/MinCover on six
existing R(x,y)-S(y,z) fixtures, with scheduling enabled for all treatments,
three timed repetitions plus one warmup. All 72 executions pass exact Out,
aggregate match, and every-round match checks. These explicit repeated-query
fixtures use --naive as well as --no-decomp. Equal-size, one-row-per-y joins
improve query time 24.6% under PureSize; MinCover's paired median is also
24.6% (mean 29.5%, influenced by one unusually fast sample). At 100:1 and
1:100 size ratios with unique y, query improves roughly 16-17% and wall
5-7%. All three pairs improve query and wall for both strategies in all
three unique-y cases. Equal fanout 8 instead loses 67-69% query time, fanout
64 loses 29-30%, and 1:100/fanout8/25%-overlap loses 59-60%, in every pair.
The native strategies therefore do have clear local wins; relation-size
imbalance alone does not distinguish them from the losing grouped joins.

Two Hardboiled rejection rules receive exact-selector follow-up in
`native-hard-selective`: seven timed repetitions plus one warmup, GJ and
scheduling controls, and each target separately under PureSize/MinCover.
All 96 runs preserve final sizes and every individual seminaive match entry
(2,699 entries for Hard32, 2,693 for Hard128). Both targets emit zero matches.
On Hard128, the convolution rule improves 7.779 -> 1.382/1.376 ms (about
82%) in every pair; total query improves 2.44%/2.00%. On Hard32 the same
rule regresses 2.278 -> 2.501/2.493 ms (9-10%) in every pair. The WMMA rule
improves about 41%/63% on Hard32/128 but saves only 0.104/0.303 ms.

`native-hard-wall-followup` uses original unchanged inputs, no report or
extra printing, 15 timed rounds plus three warmups, and matched GJ controls.
The exact convolution MinCover policy improves Hard128 wall 1.10% versus
GJ and 1.19% versus scheduled GJ (14/15 pairs faster against the latter,
both order groups improve). Hard32 costs 0.22% versus GJ and 0.46% versus
scheduled GJ. Empty stdout hashes are only a success check here; correctness
comes from the separate 96-report audit. Selected logical plans reveal one
convolution cover-order difference: Hard128 puts outer Ramp->Load before
the two Cast covers, while Hard32 does the reverse. Runtime DVO order was
not logged, so this is a plausible explanation rather than a causal proof.

The live-cover prototype passes all 167 core tests both with its global
switch off and on, including the new differential key/existence tests at
one and four workers. It is frozen separately and removed from main.
`live-cover-root11-pilot` compares four scheduled treatments: GJ and native
MinCover, each with live-cover selection off/on. Of 44 attempts, 32 complete
and 12 time out. All completed final sizes agree; live-on/off native match
vectors also agree exactly wherever both finish. No global native winner
appears. One eggcc extraction rule gets substantially worse (54 -> 491 ms)
with unchanged matches. The new selection does expose local Luminal wins
that the earlier native timeouts hid.

`live-cover-selective` isolates direct-exp-fusion and list-tail propagation
on llama, qwen, and whisper: seven timed repetitions plus one warmup, six
treatments, all with scheduling. All 144 reports match every seminaive entry
and final table size. Direct-exp live MinCover improves its target by 73.5%
on llama and 76.4% on qwen, reducing total query 2.06%/2.71%; every pair
improves. Whisper instead takes 0.703 -> 25.471 ms on that same rule, raising
whole-query time 7.68% and wall time 4.16% in every pair. Original MinCover
is slower on all three; removing dead cover requirements causes the useful
plan change only on the first two. List-tail original MinCover improves its
target 14.7%/15.5%/4.9% respectively, but saves little and live-cover selection
adds nothing. The direct-exp rule's shape alone is therefore insufficient
for selection. Next compare its positive and negative cached plans and test
refreshing only that rule as tables grow, including a GJ refresh control.

The exact direct-exp plans expose a fixed probe-direction problem
(`direct-exp-plans/plan-direction-analysis`). All live plans retain the same
eight covers and eliminate the same three list witnesses. Llama/Qwen bind
the operator's input list and probe the outer ICons output. Whisper instead
scans the outer ICons to probe that operator input, and no other cached-live
stage narrows this list cover. Dynamic stage ordering cannot reverse this
edge. Removing the old INil/tail covers also removes Whisper's previous
list-narrowing route. This is plan evidence; runtime scan counts were not
measured.

`direct-exp-refresh` recompiles only this exact rule before each ruleset
invocation, preserving last_run_at and seminaive construction. It passes
19 bridge tests; seven timed repetitions plus one warmup compare GJ and
live MinCover, each cached/refreshed. All 96 reports preserve final sizes,
individual match entries, and changed flags. Llama/Qwen target times get
slightly worse (+1.94%/+1.14%). Whisper's target improves 25.429 -> 16.052 ms
(-36.88%), query -2.50%, and process wall -1.66% against cached MinCover,
with every pair improving. It still loses query 5.20% and wall 2.98% against
refreshed GJ. Recompilation is outside core query/run timers but inside
process wall; GJ controls do not isolate compilation overhead.

Refreshed Whisper plans move dtype before ICons, enabling head-column
refinement, but retain the bad ICons-to-operator direction. Current base
statistics therefore help without fixing the strategy. No general refresh
policy is retained. The next isolated experiment promotes an already
refined cover when it can narrow the next otherwise unrefined cover,
changing the static direction without adding indexes or runtime replanning.

The bridge-cover ordering experiment promotes the smallest already-refined
remaining cover when it can introduce a variable of the next wholly
unrefined cover. It changes native ordering only, preserves independent
roots and already-refined covers, and adds no indexes or runtime statistics.
The prototype passes 168 core tests with flags off/on, including an actual
generated-probe/order test. Temporary source changes are removed after
freezing the binary.

`direct-exp-bridge` uses seven timed repetitions plus one warmup, with GJ
and native live MinCover each bridge-off/on. All 96 reports preserve final
sizes and every seminaive match/changed entry. Llama/Qwen logical plans are
unchanged. Whisper now generates OpMul -> IConsOuter, removing its third
unrefined starting cover; its stage instructions match Llama/Qwen. The
Whisper target falls 24.880 -> 1.107 ms (-95.55%), query -6.81%, wall -3.63%
against cached native, with every pair improving. But GJ's target is
0.710 ms, so even the corrected direction remains 55.9% slower there.
Whole-query/wall against matched GJ change +0.18%/+0.22%, with mixed paired
signs. This isolates the direction defect without establishing a native
strategy winner for Whisper.

`bridge-cover-root11-pilot` globally compares the same four treatments,
one diagnostic sample each. Of 44 attempts, 34 complete and 10 time out.
Native bridge-on/off complete the same six workloads; both time out on
Hard32/128, paged_llama, qwen3_moe, and whisper. All completed final sizes
agree. Every bridge-on/off individual entry vector agrees within strategy;
only the previously observed native-vs-GJ eggcc order differences remain.
The bridge repair also reduces CYK native query 354 -> 50 ms and old Luminal
73 -> 33 ms, but these still cost 2.07x/5.16x matched GJ. No whole-workload
native winner appears. Other pilot changes are small relative to the large
remaining native regressions and do not justify performance claims.

The next hybrid experiment retains scalar GJ output/anchor intersections
and makes their execution a prerequisite for one fused shared-key tuple.
Earlier key-pair fusion preserved scalar stages but allowed DVO to run the
fused scan first; a static position does not preserve the narrowing path.
The proposed scope is one cover, at least two shared primary-key variables,
full key supplied by those variables/constants, and a shared non-key anchor.
No general native switch or replanning policy is promoted from these results.

`direct-exp-bridge-wall` confirms the isolated selected-rule change on
original unmodified inputs, without reports or added printing: 15 timed
rounds plus three warmups, GJ and GJ+bridge controls. Against the matched
control, llama wall improves 0.50% (13/15 pairs) and qwen 0.57% (13/15).
Whisper changes +0.08%, effectively neutral. Against plain GJ the changes
are -0.36%/-0.88%/-0.04%. All 135 timed executions succeed; empty stdout
hashes are not a correctness oracle, supplied instead by the earlier
96-report entry audit. These are modest local process gains, not a general
strategy selector.

A small executor follow-up (`tiny-native-tuple`) reuses SmallExact for
non-inline tuple subsets of up to SMALL_RESIDUAL rows when the existing
Packed/Catalog continuation-slot test is false. It passes 168 core tests
with the flag off/on and is removed after freezing. Five timed repetitions
plus one warmup compare native bridge/GJ, each shortcut-off/on, on CYK,
old Luminal, and exact direct-exp in llama/qwen/whisper. All 120 reports
preserve sizes and every individual match/changed entry. Native whole-query
changes are +0.04%/+0.31%/+0.09%/-0.20% for the first four; the Whisper
target is unchanged (+0.05%). Apparent Whisper wall reduction is largely
outside reported execution and accompanied by a similar GJ control query
shift. No useful targeted improvement is established. The slot predicate
also admits Owned plan roots that may have reusable prepared indexes; this
limits the claim that eligible sources are uncached. No incidence counter
proved how often the new branch ran. Discard this shortcut without further
timing expansion; any revisit needs direct routing coverage and reuse data.

`native-batched-beta` isolates the two rules containing "cublaslt row-order
batched scaled c", seven timed repetitions plus one warmup on llama, qwen,
and whisper. GJ/PureSize/live MinCover share bridge ordering and scheduling.
All 72 reports preserve sizes and every individual match/changed entry.
Live MinCover reduces the combined target 42.98%/41.90%/42.06%, saving
0.337/0.339/0.521 ms; each rule improves in all seven pairs on every input.
PureSize gains only 4.05%/6.76%/3.87% combined. Whole-query Min changes
-0.79%/-0.61%/-0.72%, wall -0.08%/-0.31%/-0.51%; effects outside these
targets limit attribution, especially Whisper. Both targets have zero
matches. Saved external data likewise show zero for both in all seven
Luminal inputs across 70 observations; other suites lack these rule names.

Exact beta plans explain an important difference from PureSize. Each GJ
plan has 42 stages (35 scalar intersections, seven payload scans). PureSize
has 31 covers and 42 index accesses; seven or eight tails bind only dead
timestamps. Live MinCover has 18/19 covers and 39 index accesses, with no
timestamp-only tails. It removes 12/13 stages but only three probe slots;
all removed bindings are dead timestamps. Reduced stage setup and initial
quadratic scheduling work is plausible, not separately timed attribution.
A new narrower experiment will remove only provably redundant dead cover
stages while preserving original cover selection and probe directions.
The earlier dead-projection prototype explicitly kept otherwise-empty
stages, so this is a distinct check.

`anchored-key` groups one complete shared key in a GJ SinglePlan, retaining
scalar non-key anchors and requiring them to execute first. It passes all
171 core tests with the flag off/on and the prerequisite disabled. One
existing scalar-shape assertion is adapted only in a temporary test patch
to accept the intended two-key fused group plus scalar anchor; its exact
stale-row output/count oracle still runs. New tests cover constants, repeated
keys, incomplete/private-key rejection, slow constraints, stale rows, one/four
workers, and recursive prerequisite ordering. All prototypes are frozen
separately and removed from main.

`anchored-root11-pilot` rejects this as a broad policy. Of 44 attempts, 42
complete and both Whisper hybrid variants time out. Every completed final
size and individual entry vector agrees. Llama's zero-output 2D scaled-C
plus-matmul beta rule alone rises 0.385 -> 649.814 ms; disabling the anchor
wait still costs 639.231 ms, so grouping itself causes that loss. Python
additionally gets worse when forced to wait. Hard32's apparent whole-query
gain is not trustworthy attribution: most hot rules, single-atom rules,
merge/rebuild, and time outside the reported run all improve together in
this unwarmed first case. Hard128 does expose two positive-output local
gains (~26 -> 21 ms, 64 matches each), overshadowed by a different rule's
103 ms regression. These need exact-rule isolation before conclusions.

`elide-dead-cover` tests the narrower dead-stage idea. After native cover
selection, it omits a probe-free stage only when an earlier retained stage
has attached the atom's existence/slow constraints, every physical key
column is already fixed by a bound variable or equality constant, and all
new bindings are private, unused, non-key variables. Headers and atoms
remain. Complete-key uniqueness preserves multiplicity; private key columns,
placeholders, and isolated witnesses retain their stages. The combined
follow-up binary passes 171 core tests with flags off/on, including actual
elision, composite/repeated keys, stale rows, slow rejection, and one/four
workers. Temporary changes are removed after freezing.

`elide-dead-cover-results` compares GJ, PureSize, and live MinCover, each
elision-off/on, five timed repetitions plus one warmup. All 252 engine runs
succeed; every matched on/off final size and ordered individual match/changed
entry agrees. The nine known native-vs-GJ eggcc entry differences are present
with elision both off/on. PureSize's two beta rules improve together by
18.79%/18.76%/18.60% on llama/qwen/whisper (all five pairs), saving
0.154/0.155/0.239 ms. Whole-process time does not improve. Live MinCover
already avoids these tails and gets essentially no additional benefit.
Whisper direct-exp and CYK are neutral. Eggcc live-Min query improves 2.99%
but remains about 4.44x GJ.

A decisive negative appears in old Luminal: PureSize query rises
42.032 -> 247.019 ms, wall 138.967 -> 345.500 ms, in every pair. The
row-rms-norm rule accounts for 205.639 ms of the increase with identical
65-match vectors. Retaining logical probe directions does not preserve
physical scheduling: removing stages changes refinement priorities and
runtime re-sort positions. This rules out globally enabling the elision
without understanding that interaction. Matched plan and DVO diagnostics
are the next step, rather than expanding timing of the failed policy.

`anchored-isolated` independently selects each promising or negative hybrid
rule: five timed repetitions plus one warmup, GJ and an unmatched-selector
anchor-wait control, and waiting/non-waiting variants per target. All 240
reports preserve final sizes and every individual match/changed entry.
An independent plan audit covers 53,922 entries: only the 813 exact selected
entries change their logical plan. Hard32's broad cold-pilot gains do not
reproduce. Hard128's two positive-output convolution rules do: each drops
about 27.0 -> 21.8 ms (19.4%), with 64 matches, query -0.97%/-1.18%, wall
-1.11%/-1.37%, all five pairs. Disabling the anchor wait gives similar gains.
The negative convolution rule rises 144.882 -> 253.380 ms (+74.9%), also
with unchanged positive outputs. Both Hard sizes compile the good Tile
covers at three estimated rows and the bad Float cover at six; a cutoff
between these values would be fitted to these cases rather than explained.

The sigmoid marker improves 30-42% on llama/qwen/qwen3_moe but regresses
1.874 -> 6.761 ms (+261%) on whisper. The expert-index marker improves on
all four: 50.7%/48.2%/64.0%/9.8%, with 1/1/5/2 matches. Absolute expert
savings are only 0.253/0.243/0.802/0.068 ms, so process effects are small
and mixed. All target signs repeat in every pair, and disabling the anchor
wait gives similar results. The first three workloads choose an Op cover
with constructor/list key variables; whisper chooses ICons and reverses
the grouped edge. These are positive-output local wins and a useful
candidate-direction experiment, but no general anchored selector is ready.

`anchored-domain-rank` tests candidate direction using existing base/header
sizes: for each grouped key variable, find the smallest other real atom,
multiply its row-count bounds with saturation, and cap by the cover's own
rows. This estimates a bound on feasible complete-key tuples at compilation,
not scan cost or selectivity. It adds no indexes. All 171 core tests pass
with flags off, ranking enabled, and combined diagnostics enabled. All
16 plan diagnostics preserve 75,492 individual entries; only Whisper's
36 sigmoid and 25 expert plans change. Both now choose Op, making each
marker family's complete logical plan identical across the four models.
Whisper sigmoid changes ICons784 -> Op1028 with key domains[8,1]; expert
changes ICons844 -> Op1099 with domains[15,1]. Other model choices stay fixed.

`anchored-domain-markers` measures only those changed choices, five timed
repetitions plus one warmup. All 36 reports preserve every size/count entry.
Sigmoid improves 6.745 -> 1.769 ms (-73.77%) against the original hybrid,
and beats matched GJ's 1.838 ms by 3.70% in every pair. Its net GJ saving is
only 68 microseconds: the positive seven-match entry saves 209 us while
zero-match entries cost 141 us more. Most of the apparent whole-query
2.14 ms reduction versus GJ is unrelated-rule variation, so do not attribute
it to this choice. Expert is mixed/slightly worse than the original hybrid
(0.6165 -> 0.6254 ms), while still 7.12% faster than GJ in every target pair.

The four-case `anchored-domain-pilot` rejects ranking as a general fusion
policy. All 12 reports preserve individual entries, but both good Hard128
rules lose their gains (21.7 -> 28.3/28.8 ms), the bad Float rule remains
about 253 ms, and Llama's bad beta rule improves 652 -> 6.32 ms but remains
15.8x GJ. These are single diagnostic samples. The pilot's candidate traces
explain a stale-statistics trap: Hard128 switches Tile3 -> Cast48 because
one constraining relation has zero rows when first compiled; Llama similarly
chooses an ECons whose producer is initially empty. These cached domains
later grow. Next test treating zero domains as unknown and retaining GJ
when no candidate has an informative reduction, without a fitted size cutoff.

`elide-rms-isolation` confirms the earlier stage-removal regression belongs
to row RMS itself: three timed repetitions plus one warmup, full-native and
exact-rule-only controls. All 16 reports preserve 1,155 ordered entries and
65 target matches. Exact-only Pure goes 7.557 -> 206.834 ms; full-native
7.657 -> 206.256 ms. Changes to other rules are not required for the loss.

Native DVO diagnostics separate executed-prefix refinement, stable logical
ties, recursive sorting disabled, and sorting at every eligible position.
These are heuristic ablations, not semantic fixes: repository history already
contains physical-prefix changes and reversions, and current tests protect
the logical baseline. Two new focused tests cover actual fused prefix
selection and unchanged ordinary GJ behavior. A fixture initially tied an
unrelated cover with the intended target; its size was corrected rather than
weakening the assertion. All 173 core tests then pass with flags off,
physical-prefix enabled, and all diagnostic policies enabled.

The bounded RMS trace shows the first important recursive difference at
position four with identical retained residuals. Elision changes logical
prefix credits, promoting Cast before F32. A 65-row Sum scan consequently
moves from position seven, where the original modulo-three policy re-sorts,
to position six, where it scans before a useful filter. Stable tie-breaking
alone does not repair it. Both skipping recursive sorting and using executed
prefixes avoid the blowup but make the original unelided plan slower.

`rms-dvo-confirm` repeats the promising every-position policy five times
plus one warmup with GJ controls. All 36 reports preserve every size/count
entry. Pure's target improves 7.834 -> 5.433 ms (-30.65%), whole query
-20.78%, wall -3.17%, every pair and both order groups. Elided Pure improves
214.747 -> 5.460 ms. Elision adds no demonstrated benefit once cadence is
fixed. GJ's target is still 2.201 ms: the repaired native plans remain about
2.47x slower on the target and 2.7-2.9% slower in process wall. A broader
bounded native screen is needed before drawing any general scheduling claim.

`native-every-root11-pilot` broadens the cadence comparison to GJ, PureSize,
and live MinCover, each default/every, one timed sample and a ten-second
watchdog. Of 66 attempts, 50 succeed and 16 time out. All 14 completed
native cadence pairs and all 11 GJ cadence pairs preserve final sizes and
every ordered individual count/changed entry. Hard32 Pure differs from GJ
in 13 entries, identically with cadence off/on, in addition to the previously
known eggcc cross-strategy differences. Those are separate from the marginal
cadence change; no new cadence-induced count differences appear.

Cadence repairs large native costs beyond RMS: Pure query Llama
6.559 -> 0.514 s (-92.2%), Qwen 9.283 -> 0.731 s (-92.1%), paged Llama
6.761 -> 1.113 s (-83.5%). Live MinCover Llama goes 2.486 -> 0.497 s,
Qwen 3.824 -> 0.733 s. Positive-output pair-fuse/grow rules contribute to
these savings. No completed native workload beats GJ, and no timeout is
rescued. The narrower follow-up retains the original cadence and additionally
re-sorts before a large filtered FusedIntersect, excluding position zero and
the last stage. It tests only the current instruction, preserving ordinary
scalar GJ and scan-only-tail cadence without scanning the whole plan.

`anchored-bound-admission` treats a zero key-domain estimate as unknown,
falling back to the cover's own rows, then admits the selected candidate only
if its bound strictly reduces that estimate. It passes all 172 core tests
with flags off/on; synthetic semantic fixtures use explicit informative
estimates so they still exercise fusion, while a new fallback test checks
exact ordinary GJ stages and variable/constraint state for rejected cases.
No state changes occur before admission. All temporary source is removed
after freezing.

The exact Llama beta safety check still fails on cost: GJ 0.488 ms,
previous domain ranking 7.785 ms, admission 6.199 ms. Apparent whole-process
improvement is unrelated cold/order variation. `bound-admission-root11-pilot`
completes all 44 attempts and preserves every individual count/changed entry
and final size. Nevertheless waiting admission raises query time about
73%/80%/85%/91%/45% on Llama/paged/Qwen/Qwen3/Whisper against its matched
GJ control. No-wait also regresses substantially; the other six workloads
are roughly neutral in these single samples. The policy remains experimental.

Five exact loss-plan traces identify a stable structural cause rather than
an accidental row-count-one estimate. They select ECons(MIter, ENil) or
ECons(MNum(0), ENil), each with key domains[1,1], while the ECons cover has
116 or 188 rows. The domain atoms have zero physical keys or complete keys
fixed by equality constants. Scalar GJ can therefore bind these keys from
singleton sources before probing the list relation; grouping them into a
cover scan discards that cheap access path. The next admission ablation
rejects a selected group whose every key has such a provably singleton
source, keeping ranking unchanged and avoiding a numeric product threshold.


## Filtered fused re-sort cadence confirmation

The narrower scheduling policy keeps the modulo-three baseline and also
re-sorts before a large current filtered FusedIntersect, excluding the first
and final positions. It uses one current-stage check; ordinary scalar GJ and
unfiltered payload tails retain their previous cadence. It does not change
logical-prefix refinement, ordering keys, barriers, caches, or leaf handling.
The production candidate is nine added lines and one removed line in
execute.rs, with no diagnostic flags.

`filtered-dvo-confirm` runs five timed repetitions plus one warmup for seven
cases. All 198 reports preserve complete ordered per-rule count/changed
entries and final sizes within each strategy across cadence variants and
elision. Pure Llama query improves 6.614576 -> 0.518936 s (-92.15%), Qwen
9.343010 -> 0.729709 s (-92.19%), and Hard32 0.750118 -> 0.383378 s
(-48.89%). Live MinCover Llama improves 2.482997 -> 0.492040 s (-80.18%),
Qwen 3.778445 -> 0.747323 s (-80.22%), eggcc 0.671895 -> 0.636997 s
(-5.19%). The filtered policy retains essentially all of the every-position
policy's tensor gains; the initial MinCover Llama and eggcc losses against
that policy do not repeat. MinCover Qwen's +1.21% mean versus every-position
comes from one +9.24% sample, with median -0.033% and three of five faster.

RMS's unelided target falls about 29%, and elided Pure's 213.449 ms falls
to 5.555 ms, but GJ remains faster at 2.268 ms. All these whole native
workloads remain slower than GJ; some individual native rules regress even
when total query time improves greatly. This is a fused-path scheduling
improvement, not evidence to replace the default planner globally.

The production-only frozen binary passes all 166 core and 19 bridge tests
and formatting. `filtered-production-root11` uses unchanged original inputs,
15 timed repetitions and three warmups, CPU24, one worker, --no-decomp.
All 330 timed processes succeed and stdout hashes match (only Python prints
nonempty stdout, so this alone is not the detailed count oracle). Arithmetic
mean wall change is -0.027%, sum of mean times -0.036%: neutral overall.
CYK improves 1.12% in all 15 pairs. Qwen's +0.67% cost repeats in 14/15
pairs and both order groups; paged Llama +0.24%, Qwen3 +0.44%, and Luminal
+0.41% are also small costs rather than an across-the-board win.

`filtered-production-external81` completes all 648 runs (three timed plus
one warmup per variant). Independent audits verify exact final-size maps,
printed per-rule count multisets, and binary/input/adapter/runner hashes.
Sum of mean wall times improves 0.375%; geomean mean time improves 0.180%.
Fifty-seven of 81 cases and 159/243 pairs are faster. Suite sums improve
1.234% Luminal, 0.177% eggcc, and 0.242% Hardboiled. Perfect's +3.86% mean
contains one +12.07% outlier outside the rounded query/merge/rebuild phases;
conv1d_224's +0.97% is smaller but appears in all three pairs and query time.
These receive focused follow-ups. The initial math12 run preserves all
reported results and has query -0.10%, but wall +2.13%, driven by one
8.797 s baseline versus other baselines 9.250/9.340 s; a repeated comparison
is required before deciding whether to retain the patch.

The fixed-key admission prototype is also frozen, with all 173 core tests
passing in flags-off, admission, and combined configurations. It rejects a
selected group when every grouped key is supplied by another real atom
whose complete physical key is explicit constants or empty. It excludes
the chosen cover, does not use row-count-one estimates, preserves ranking,
and falls back before mutating planning state. Tests cover partial versus
complete composite constants, zero-key sources, estimates being insufficient,
self-exclusion, and exact GJ planning-state fallback. Performance is pending.


The focused production checks resolve those concerns without hiding their
variation. Perfect's eight timed repetitions plus one warmup preserve all
18 size/count reports; mean wall +2.42% but median +0.028% and paired
median -0.140%, with four of eight faster. Both binaries share the early
95-96 ms execution state before later 77-80 ms runs; CPU times also change,
while RSS stays similar. The cause is unestablished. Conv1d_224's eight
pairs preserve all reports and yield wall -0.194%, query -0.198%, with both
order groups about -0.19%; the earlier loss does not persist.

Math12's five-pair, two-warmup follow-up preserves all 14 final-size maps
and 352 ordered entries/changed flags in each report. Query is
1.98714 -> 1.97949 s (-0.385%), wall 9.10426 -> 9.03817 s (-0.726%).
Only two of five query pairs improve and order groups differ in wall time,
so this supports neutral performance, not a reliable new math speedup.
The first three-pair run's wall gap was mainly iteration-12 rebuild and
merge, with no query regression; similarly fast runs occur for both binaries
in the follow-up. No resource cause is asserted for math because its
harness does not record those counters.

Keep the small filtered-cadence change: it repairs substantial repeated
native fused costs, remains neutral overall on default root11/external81/
math12, and has exact detailed count validation. Retain the documented
sub-percent default costs, including root Qwen, as tradeoffs. The retained
patch adds only an explanatory comment to the measured condition; no
experimental strategy overrides, admission rules, or flags are promoted.


## Fixed singleton-source rejection and exact Ramp isolation

`fixed-keys-beta-safety` (15 with-plan diagnostic runs) and
`fixed-keys-root11-pilot` (55 bounded one-sample attempts) all complete.
Independent audit reads all 70 reports and 427,912 entries: ordered counts,
changed flags, aggregate counts, and final sizes agree. All five fixed beta
plans exactly reproduce GJ, while the prior policy changes only the selected
rule and actually emits filtered fusion. The frozen binary predates the
retained cadence patch; this isolates the admission change.

Rejecting structurally singleton keys removes 86.5-93.6% of the prior
large-model query penalty, including an untargeted paged FlashInfer loss
(34.05 -> 17.52 ms, GJ 17.66 ms). Sigmoid/expert local gains remain.
Nevertheless fixed-wait still loses to matched GJ: Llama +5.8%, paged
+5.2%, Qwen +7.2%, Qwen3 +11.9%, Whisper +4.9% query. Residual cublaslt
cost is about 4.0/4.3/4.8/10.2/14.7 ms across those workloads, chiefly
zero-output batched layout rules. No-wait does not remove the problem.
The other six cases are broadly neutral in this discovery sample. Keep
this prototype unpromoted and inspect residual selected covers before
extending the proof or adding another estimate-based gate.

`ramp-arithmetic-isolation` selects each of three exact rules independently
on Hard32 and Hard128, GJ/Pure/liveMin with payload deferral, bridge ordering,
and filtered cadence. Five timed repetitions and one warmup yield 108
successful reports; every ordered entry/changed flag and final size matches
GJ. Equal-lanes Pure is consistently faster locally: Hard32
1.214 -> 1.010 ms (-16.8%), Hard128 3.068 -> 2.399 ms (-21.8%), all five
pairs and both order groups. Whole-query changes are +0.29%/-0.22%, wall
+0.06%/-0.13%: too small to claim meaningful workload acceleration.

The similar ramp-pair rule is +4.5% on Hard32 but -3.2% on Hard128, all
pairs in each direction. Divisible-lanes is +8.5%/+2.4%, every pair slower.
Live MinCover is substantially worse: equal-lanes 24.93/160.44 ms, ramp-pair
31.30/167.78 ms, divisible-lanes about 3.8/19.67 ms. This excludes a blanket
three-rule admission policy. Equal-lanes is the replicated Pure candidate;
plan traces must identify which grouping and orientation produce the gain.
Unlike the first two rules, divisible-lanes has no relationally shared lane:
its lane variables meet only in a primitive condition.


Untimed residual diagnostics (`fixed-residual-loss-plans`, three GJ/fixed
pairs) identify MMul(MIter,k) in all three batched layout patterns. Its
own estimate is 450, key domains [1,20], bound 20. The second key varies
through Sum; this is not a missed nested-constant proof. The fused stage
probes Sum on both grouped keys. In contrast, the eight model marker plans
that improved probe other atoms on only one grouped key at a time. A narrow
next ablation leaves a proven-fixed key scalar when another probe contains
both it and a varying grouped key. The two-key case can return exact GJ;
higher-arity cases can retain fusion of the remaining varying keys.

The six `ramp-equal-lanes-plans` captures also revise the hypothesized cause.
Pure first restricts the operator through AddOrSub, then scans Bop to bind
both constructor outputs, probes Ramp/Broadcast on those outputs, and only
then joins their lanes through Broadcast. Live MinCover omits AddOrSub as a
cover and starts Broadcast/Ramp, delaying the operator restriction until
last. Plans are identical across sizes and counts agree. The next useful
ablation is Bop's remaining two shared key columns after preserving the
unary-domain operator as a scalar key anchor. Private Ramp-key fusion is
not the demonstrated cause of this PureSize gain.


## Splitting fixed keys before compound probes; asymmetric binary fanout

`anchored-split-fixed` factors the existing fixed-source proof into a helper.
For the already-ranked candidate, it leaves a proven-fixed key scalar only
when another real atom contains that key and a varying grouped key. With
fewer than two varying keys left, it returns the exact original GJ plan
before mutation. With at least two, only those keys form the group; existing
anchor waiting preserves the scalar refinement first. It leaves ranking and
separate one-key probes unchanged. All 175 core tests pass with flags off,
split policy enabled, and combined diagnostics. Tests include both fallback
and retained higher-arity fusion, repeated columns, self-exclusion, constants,
stale cover/probe rows, slow constraints, and one/four-worker output oracles.
The new frozen binary includes the retained production cadence in all controls.

Six exact residual-plan captures pass: each 6,040-entry GJ/split pair has
identical counts, changed flags, final sizes, and every logical plan. A
collector audit path initially used the old label `fixed` instead of `split`;
the six engine runs had succeeded, and an audit-only correction validates the
saved artifacts without repeating them.

`split-fixed-root11-pilot` completes all 55 attempts and preserves all 350,590
ordered report entries, changed flags, and sizes. Query changes versus the
matched waiting GJ control are Llama -1.91%, paged -1.35%, Qwen -1.86%,
Qwen3 -3.06%, Whisper +0.92%. The three diagnosed batched layout losses
vanish. Residual costs include the fourth col-by-row layout orientation and
Whisper unroll-Add/direct-exp. Hard32's large pilot gain spans nearly every
rule and is not attributed to fusion; repeated confirmation is required.
The five-pair, one-warmup root11 confirmation has completed; independent
paired/count audits are pending. Preliminary wall times remain mostly
neutral or mixed, so no planner change is promoted.

`asymmetric-binary-pilot` tests eight ordinary-relation fixtures: fanout 1:8
and 2:16, both mirrored and both row-size orientations (4096:8192 and
8192:4096). Each performs exactly 524,288 actions across repeated naive
rounds. Separate frozen-binary panels compare forced first/last/small/cached-
unique directions with their own GJ control, and native Pure/Min with their
own GJ control. No raw timing comparison is made between binaries. All 64
runs, 4,608 individual ordered rule entries, aggregate 33,554,432 actions,
and final output sizes agree; all frozen/input hashes pass.

Mirrors agree: the lower-fanout cover is always cheaper than the opposite
fused direction, but a larger unique cover still loses to GJ by about
36-44%, despite beating the smaller high-fanout scan (about 141-146% slower
than GJ). Even a smaller fanout-two cover loses about 14-15%. Only a unique,
smaller cover has a modest historical forced-path win in this discovery
sample. Native Pure/Min lose all eight cases. This supplies no reason to
replace GJ with a scan of a larger unique cover. Existing GJ already chooses
the smaller distinct-key prober after index construction; row count is not
its sole choice criterion.

Omitting private unused non-key/timestamp bindings while retaining stages
and probes was already tested separately (`strategy-prune-dead.patch`), not
just dead-stage elision. It improved native binary-MinCover math query about
3.23% but remained 7.46% slower than GJ. Different historical leaf-borrowing
and newer factor batching also confound any cross-binary explanation of
the asymmetric panel gap. Do not reattribute that gap to timestamp columns
or rerun projection pruning without new evidence.


The completed split confirmation preserves all 264 reports, 1,682,832
ordered individual entries, changed flags, aggregate counts, final sizes,
and frozen/input hashes. Split-wait versus matched waiting GJ improves
query time in all five pairs and both order groups for Llama -1.94%, paged
-1.08%, Qwen -2.11%, Qwen3 -2.89%. Corresponding wall changes are +0.61%,
-0.91%, +0.70%, +0.19%. Whisper query +0.19% is mixed by order group.
No-wait versus plain GJ also has all-five query gains on those four models,
with generally neutral wall and Qwen3 -0.73%. These results establish small
query improvements, not a broad end-to-end gain. Keep the policy experimental.

Six further Whisper plan captures preserve all 36,240 entries and change
only the selected rules. The remaining col-by-row layout selects
MMul(MIter,nth_from_end87), domains [1,161], with independent scalar probes;
it is not the repaired Sum compound-probe pattern. Unroll-Add selects
ICons(LoopStart,INil), domains [2,1], but LoopStart's physical keys vary, so
its estimate of two is not a singleton proof. Direct-exp selects
Op(Constant,INil), domains [8,1], with a range-constrained Constant and scalar
probes. That shape wins on other models. No further structural rejection
is justified by these traces alone.

Timer inspection distinguishes known boundaries from speculation: cached
plan creation/admission and seminaive header assembly precede db.run_rule_set
and are absent from rule timing. Prepared index metadata and lazy root/
residual index acquisition are inside serial rule query timing. In the four
improving models, the difference between search/apply and summed rule queries
changes only about -0.026 to +0.070 ms, while reported execution phases improve
0.77-1.69 ms. Mixed wall changes sit in a much larger unreported bucket that
also includes frontend, input, serialization, startup, and teardown. Existing
reports cannot attribute that bucket to admission or index construction.

## Conditional full-key fusion after an operator-domain filter

The separate `anchored-pk-domain` prototype starts from the original anchored
stack, omitting later domain-bound/fixed-key/split policies. A candidate's
complete physical key must consist of shared variables or explicit constants.
One key variable must have another real unary-key domain source with no other
live variables. Leave that variable scalar and group at least two remaining
shared key variables; constants plus scalar anchor plus group still cover the
full physical key. This allows a private RHS output, unlike the old non-key
anchor requirement. Existing anchor waiting needs no change. All 173 core
tests pass flags-off, PK-domain, and combined diagnostics; fixtures cover
repeated/constant/missing keys, fallback state, stale rows, multiple operator
values for the same key pair, payload fanout, and one/four-worker output.

`pk-domain-equal-lanes-screen` first validates six untimed GJ/Pure/PK plans,
then runs five timed repetitions plus one warmup on Hard32/128. All 42 reports
preserve full counts, changed flags, and sizes; only the selected logical
plans change. The candidate is exactly Bop(lhs,rhs) fusion with retained
operator and lane intersections and three payload leaves. It captures the
Pure local gain: Hard32 GJ 1.134 -> PK 0.948 ms (-16.4%), Pure 0.953 ms;
Hard128 2.882 -> 2.258 ms (-21.7%), Pure 2.294 ms, all five pairs. Whole-query
+0.956%/+0.205% and wall +0.616%/+0.255% do not establish a workload gain;
order groups differ outside the target. A bounded broad eligibility screen
is the next experiment, not an immediate production change.


Further attribution prevents treating the PK whole-query tradeoff as mere
noise. The unselected ramp-pair rule slows +16.7%/+16.5% on Hard32/128,
in every pair and both order groups, despite identical logical plans and
counts. Its extra 0.162/0.395 ms offsets most of the selected rule's
0.186/0.624 ms saving. Changed row/action ordering or cache behavior is a
plausible downstream cause, not an established one. PK and Pure per-entry
target savings correlate 0.997/0.996, supporting the same local mechanism.

Final split attribution is similarly mixed: four known positive-output rule
families save 1.566/1.107/1.698/2.662 ms in Llama/paged/Qwen/Qwen3, with each
rule faster in all five pairs, while other rules add back
0.803/0.475/0.791/0.883 ms, chiefly the two remaining col-by-row layouts.
Whisper has consistent losses in unroll-Add4 (+1.622 ms), direct-exp
(+0.487 ms), and both col-by-row variants (+0.482/+0.427 ms); mixed lower
FEFE times offset these in the total. Summed split-wait query is -0.145%
against wait-only GJ but +0.079% against plain GJ; summed wall
+0.008%/+0.252%. No broad wall win is established.

Constant-header processing is another timer-boundary nuance: seminaive
reprocess_constraints can construct column indexes before db.run_rule_set.
A planned diagnostic will time cached-plan construction and this header
phase separately. The absence of ordinary join-index work from the wall
residual does not imply that all index construction is inside query timing.


## Broad PK-domain census and first-use planning cost

`pk-domain-root11-pilot` completes all 33 attempts and preserves all 210,354
ordered entries, changed flags, and final sizes. There is no catastrophic
regression, but one-sample controls vary substantially (including Hard128
and eggcc), so small whole-workload changes do not establish performance.
The compact `pk-domain-selection` census reuses frozen inputs/binary and
associates the synchronous selected-cover trace with its immediately preceding
rule-description trace. All eleven untimed reports reproduce the pilot's full
ordered counts/changed flags. Only the three Ramp rules in each Hardboiled
case and Gamma/Theta purity rules in eggcc accept groups. Every model, Luminal,
Python, and CYK selects none; their pilot timing changes are not attributed
to fused execution.

Jointly changing the three Ramp rules removes the earlier unchanged
ramp-pair penalty: ramp-pair saves 0.074/0.282 ms, and the three targets together
save 0.227/0.658 ms (6.34%/9.11%) versus the waiting control. Divisible-lanes
still costs 0.065/0.198 ms more. The first two rules close a relational triangle
because the constructor children share a lane. Divisible-lanes is relationally
a path; its lane condition exists only in a primitive. The eggcc purity rules
are independent unary-probe stars. The next ablation will require a genuine
child-to-child relational edge, with cheap necessary key-arity checks before
candidate analysis. This is a hypothesis, not a cost guarantee.

`planning-phase-trace` is a temporary bridge-only instrument, never promoted.
The frozen split diagnostic passes all 175 core tests in three configurations
plus 19 bridge tests. Five models, GJ-wait/split-wait, three runs plus one
warmup give 40 reports and 181,440 exact ordered entries, changed flags,
aggregate counts, and final sizes. Timings are explicitly diagnostic.
Each report iteration is aligned to its trace using exact search/apply ns,
merge ns, and executed-plan count in order. Reported schedules compile exactly
491 cached plans in every model/treatment. Extra traces cover 1,245-2,692
one-shot commands/actions absent from RunReport; those are accounted separately.
No ruleset-based rebuild calls occur, so nested rebuild timers do not double
count these native rebuilds.

Split adds first-use cached compilation 0.675/0.617/0.569/0.509/0.593 ms in
Llama/paged/Qwen/Qwen3/Whisper. All fifteen paired compile differences are
positive. Schedule header/preparation differences are
-0.024/-0.015/-0.139/+0.014/+0.012 ms; unreported executor differences
+0.006/+0.034/+0.012/+0.036/-0.012 ms. The repeatable outside-query penalty
is total first-use compilation, not repeated constant-header indexes or
hidden join-index setup. This does not isolate the admission helper alone.
A minimal follow-up skips atoms with fewer than two physical keys before
looking for a two-variable group, preserving eligibility and ranking.


`anchored-min-arity` tests that necessary check without changing candidate
eligibility, ranking, or state. Its frozen diagnostic passes all 175 core
tests in three configurations plus 19 bridge tests. The three-control,
three-repetition/one-warmup phase comparison completes all 60 reports and
preserves all 272,160 ordered entries, changed flags, counts, sizes, hashes,
and control environments. Each reported schedule still compiles 491 plans.
Arity-minus-old compile means are +0.0170/-0.0417/-0.0393/+0.0409/+0.0307 ms
across the five models; only seven of fifteen pairs improve, pooled mean
+0.00152 ms and paired median +0.02133 ms. Both split variants retain about
+0.55 ms compilation versus matched GJ, every pair positive. The check is
logically harmless but demonstrates no cost reduction. No production change
or wall-time gain is claimed from this diagnostic.

The PK selection audit accounts for 70,118 exact entries in its eleven
traces. Eggcc Gamma purity costs 0.045072 -> 0.073164 ms over 29 zero-output
entries; Theta purity changes 0.037997 -> 0.037196 ms over 36 zero-output
entries. Their combined +0.027291 ms explains little of the pilot's unrelated
whole-query +1.699485 ms. Both are stars without a relational edge between
child probes and will be excluded by the triangle ablation. Expected admitted
rules are then only equal-lanes and ramp-pair in each Hardboiled input.


## Triangle admission exposes cross-rule costs

PK-domain triangle confirmation completes 36 timing/warmup reports (97,056
ordered entries) and six plan diagnostics (16,176 entries), with exact counts,
changed flags and final sizes. The separate root11 census adds 70,118 exact
entries and admits only equal-lanes plus ramp-pair in each Hard size. Same
frozen binary, five timed/one warmup, CPU24, -j1 and --no-decomp; captured input,
binary and environment provenance is exact. V2's accepted plans equal v1's;
divisible and all other unselected plans equal GJ.

V2 retains equal-lanes gains of 17.15%/21.31% and ramp-pair gains of
8.69%/13.80% on Hard32/128, each all five pairs in both order groups. But
unchanged-plan divisible slows 16.20%/16.86% versus GJ, also every pair and
both orders, versus only 4.40%/3.42% loss when v1 fuses all three. V2's two
selected rules save .284/.949ms; divisible returns .136/.374ms and remaining
rules .212/.466ms. Whole query +.117%/-.036% and wall -.005%/-.031% remain
neutral. Rejecting a plan based on its isolated/original-context cost does
not compose into a better workload policy here.

Divisible loss persists in 25/26 and 26/26 positive-output entry means;
25 entries per size lose in all five pairs. Initial invocation contributes
only about1% of the penalty. Grouped Bop cover scans remove Bop[1]/[2] indexed
requests while retaining child-output, operator and lane requests. Lazy
catalog refresh and per-invocation packed descendants can be shared across
rules, so shifted index work is a testable explanation; existing reports do
not establish it or any row-order/cache cause. The next bounded diagnostic
counts rows/time only inside actual shared catalog refresh closures, scoped
to serial rules. Packed residual builds remain outside that diagnostic.
No production gate or holdout-wide performance claim follows from this run.

The frozen v2 build passes all 174 core tests in four configurations (off,
v1, triangle, and combined diagnostics) plus formatting. Its early physical
and distinct-key arity exits are necessary conditions shared by both timed
v1/v2 labels. The new-binary triangle0/triangle1 comparison therefore isolates
the topology gate and its changed selection, not those early exits. The
original user patch is preserved exactly after restoring source.

A separate archive review rejects speculative extension of leaf batching to
arbitrary factor counts for now. No earlier implementation of a generic
prefix walker feeding the final two factors to the existing bulk helper was
found. However, all 352 saved math12 plans permit at most two multirow leaf
factors: apparent third leaves project only outputs after their complete
primary key is fixed. This is a structural bound, not runtime telemetry.
Gemma/Whisper runtime census has zero 3+ factor calls; Hard128 has just 118
calls and 2,268 rows (1.8% of emitted rows), with no isolated cost measurement.
There is no measured material opportunity that justifies adding the path.


## Shared catalog refresh does not explain the Ramp interaction

The temporary `catalog-refresh-trace` patch records only existing
get_or_update refresh closures, with per-rule count, physical stale-row range,
and refresh duration grouped by table identity/name and ordered columns. It
creates no extra index or lookup. Serial scopes save/restore nested state;
printing follows the original query timer. Planning/header work outside the
scope and packed residual builds are excluded; RHS helpers executed in the
scope can contribute events. Closure counts include zero-row no-op refreshes.
This diagnostic is not a performance treatment: event aggregation and timers
add execution overhead even though output happens outside the query timer.

The frozen build passes all 174 core tests both disabled and with trace plus
triangle enabled, and formatting. Six GJ-wait/v1/v2 Hard32/128 runs preserve
all 16,176 ordered entries, changed flags, updated/can_stop flags, and sizes.
Independent audit aligns each of 61 nonempty reported iterations per run by
exact description, ordinal, and elapsed ns; 17 empty iterations are separate.
The 39/63 auxiliary one-shot invocations all have one entry and zero refreshes.
Input/binary hashes and control environments match their manifests.

Divisible-lanes and ramp-pair have zero shared-catalog refresh events in every
control. Equal-lanes has exactly one Bop[0] operator-column refresh, scanning
486/1,796 rows at the two sizes, unchanged across treatments. There are no
Bop[1]/[2] catalog refreshes in any of the three Ramp rules. Whole reported
workload counts/rows also match: 175/13,066 for Hard32 and 188/41,495 for
Hard128. Thus shifted shared-catalog refresh does not explain the measured
recurring divisible penalty. This does not establish a packed-index, row-order,
or CPU-cache explanation; those remain unmeasured. Source is restored and the
diagnostic is not promoted.

A separate six-run exact-rule plan collector resolves the remaining small
native list-tail win. Llama/Qwen/Whisper all use the same MinCover direction:
scan contains(tail,item,return,timestamp), probe ICons.tail, then scan the
ICons payload. GJ intersects distinct tail keys and has separate ICons.list
and contains.item leaves. All 27,680 ordered entries and final sizes match;
only the selected rule's logical plans differ, and historical GJ count vectors
also match. This confirms avoiding the contains-side index while repeating
ICons probes per contains row. It does not justify blanket admission: the
archived Whisper second delta still loses 38.59 -> 62.36 us despite its
aggregate target win. No absent-key cache or new selection rule is implemented.


## PK fusion does not generalize to resize

A bounded denoise/rec_filter/resize pilot exercises graph shapes beyond the
two convolution inputs. Its strict count oracle fails across GJ/native plans:
16/14/16 entry differences respectively, involving two Ramp and two type-union
rules (rec_filter includes six missing zero-match entries). All 101 final table
sizes and per-iteration changed flags agree. This is not classified as the
user's known eggcc nondeterminism. A separate untimed oracle appends every
existing commented output-placeholder extraction: 81/83/40 roots. All 816
extractions across four controls have identical costs and expression strings,
and every same-treatment count vector agrees with the pilot. This establishes
agreement for all declared output roots, not full-egraph equivalence.

PK-domain held-out repeat: three timed + two warmup runs per treatment, four same-binary controls, CPU24/j1/no-decomp

All60 engine runs succeeded. Independent raw audit covered174,020 ordered entries and verified frozen binary f52a1034007e1d31a887eb39c4571d61a39bfcfe0affae1dad72b60a1a66dc9a, exact source-plus-print suffixes, matching inputs and environments. Every run matches its own treatment's original pilot count vector, changed flags, aggregate counts, updated/can_stop and final sizes. GJ and matched GJ-wait agree exactly; v1 and v2 agree exactly. Both native treatments retain the same16/14/16 GJ entry differences on denoise/rec_filter/resize, respectively; these are not hidden or described as a strict cross-strategy count pass. The separate untimed oracle already showed identical chosen expressions and costs for all204 program output placeholders across all4 treatments, but it does not establish full-egraph equivalence. No extraction commands were timed.

Mean changes versus matched GJ-wait (v1 / v2):
  denoise: query +0.454% / +0.522%; wall +0.216% / +0.013%.
  rec_filter: query -0.116% / +0.354%; wall -1.280% / -1.259%.
  resize: query +2.589% / +2.528%; wall +1.469% / +0.750%.

Denoise's earlier apparent20% benefit disappears after warmups. The first two Ramp rules still save0.466/0.418ms, but divisible-lanes adds0.204/0.355ms and other rules offset the remaining gain. Both native whole-query times are slower in all three timed pairs and both order groups, although the effect is small. Rec_filter is approximately neutral at the whole-query level; the apparent wall gain is driven by one257.2ms GJ-wait sample versus247.1/248.9ms for its other samples. Both candidates beat that control only1/3 wall pairs, with order-group reversal. Ramp/Ramp loses5.31%/4.70% with its complete count vector unchanged; divisible-lanes loses1.05%/17.52% with the documented native count changes.

Resize is the consistent adverse transfer: both native candidates lose in all three query and wall pairs, in both execution-order groups. All three Ramp rules are slower. Ramp/Ramp, whose per-entry counts are exact, loses16.85%/16.66% (0.803/0.794ms). The complete trio adds1.314/1.450ms, accounting for81.0%/91.5% of total query penalties1.622/1.584ms. Thus the selected-rule timing change, not only unrelated wall noise, explains most of the regression. We have no held-out with-plan report proving that no additional rule plans changed, so this is timing attribution to named measured rules, not exclusive causal attribution to a single planner change.

v2 remains weaker than v1 across the three-Ramp total: +1.99% denoise (3/3), +4.68% rec_filter (3/3), +1.05% resize (2/3). The triangle criterion neither improves overall performance nor ensures that retained Ramp/Ramp benefits transfer between graph shapes. These small samples do not estimate narrow confidence intervals, but the repeated held-out resize loss and absent overall wins provide no basis for promotion or a costly broad external confirmation. Further work should require a concrete explanation of the adverse transfer, rather than another workload-specific topology filter.

Detailed paired samples, both order groups, per-rule timings and explicit count differences: independent-heldout-audit.json / .txt. Three timed rounds give2 control-first and1 candidate-first observation for each pair.


## Shared packed-node construction explains the ownership shift

The frozen temporary packed diagnostic passes all 174 core tests both off
and with catalog/packed/triangle flags, plus formatting. It records actual
scalar build_packed_node calls inside existing factory/cache initialization
boundaries. No additional index lookup, warming, probe counting, or publication
is introduced. Source is restored exactly; no diagnostic code is promoted.

The packed-build diagnostic confirms a recurring transfer of shared scalar
construction work. All six reports/16,176 ordered entries retain exact matches,
changed flags, aggregates and final sizes against their untraced matched
references; binary/input hashes and environments pass. Report alignment covers
61 nonempty iterations per run, with 17 empty iterations and 39/63 auxiliary
invocations kept separate. No auxiliary scalar builds occur.

GJ equal-lanes builds Bop[1]/Bop[2]; v1 and v2 equal-lanes build neither. GJ and
v1 divisible-lanes build no scalar packed nodes, but v2's unchanged divisible
GJ plan now constructs these operands. All 34 (iteration, ordinal) signatures
match GJ equal-lanes versus v2 divisible-lanes exactly after omitting measured
nanoseconds: physical column, catalog_child/packed_child route, access/family/
slot metadata, dynamic-five-family shared shape, build count, input/retained
rows, and distinct keys. There are 28 nonempty transfer entries on Hard32 and
29 on Hard128, from iteration 17 through 69. This is recurring work ownership,
not just first-invocation cost. The trace does not record row IDs/pointers, so
it establishes matching measured construction work, not pointer identity.

Transferred Bop operand work (builds / input rows / distinct keys):
  Hard32:  [1] 76 /4543 /2313;   [2] 1346 /5182 /3896.
  Hard128: [1] 62 /11783 /5840;  [2] 3652 /12119 /9648.
Input and retained rows are equal in all these operand events. All events use
shared table-family slots with Dynamic{families:5}; most are packed descendants,
with a small catalog-continuation portion. Divisible additionally builds two
one-row Bop[0] nodes in each size. Its clocked scalar construction is
0.131037ms/0.308080ms in these diagnostics. These values are NOT an untraced
performance estimate or proof of the fraction of the measured 16.2%/16.9%
penalty: clocks/aggregation perturb execution and slot allocation, cache hits,
wait/publication, tuple interiors and probing are outside the timer.

The shared catalog attribution remains exactly unchanged by owner, entry,
column, refresh count and physical row span. Thus the previous negative
catalog finding and this positive packed finding concern different layers.

Cross-rule effects extend beyond the three Ramp rules. Relative to v1, v2's
new divisible builds reduce construction charged to four later unchanged
rules: two vector-reduction/matmul patterns plus IntImm Add and Sub rewrites.
Those four save 31/34 scalar builds and 4251/11567 input rows at32/128, offsetting
part of divisible's 1424/3716 builds and 9727/23904 rows. Net v2 versus v1 is
+1393/+3682 scalar builds and+5476/+12337 input rows. Across the whole reported
run, GJ/v1/v2 build counts are 29821/29010/30403 at32 and 92031/89534/93216 at128;
input rows 321901/312595/318071 and 940848/912171/924508. V2 scans fewer packed
input rows than GJ but constructs more tiny nodes. These are work counters,
not an independent claim of speed.

Interpretation: a topology veto judged by isolated rule cost ignores reuse
between rules. Fusing earlier producers removes shared index construction
that a later unchanged consumer formerly reused, relocating its cost into the
consumer's timer. The measured ownership transfer now supports this concrete
mechanism; it does not establish that every whole-query timing difference has
that cause. Keep the whole-query neutral result and do not promote the v2
triangle veto from local target wins. If another bounded experiment is useful,
the missing divisible-only cell in the existing four-way none/two/all-three
interaction design is sufficient; no new cache or planner machinery is needed.

Artifacts: independent-packed-audit.json/.txt provides the complete report and
per-rule/entry audit. packed-transfer-attribution.json/.txt records the exact
34-entry correspondence, source/family signatures and other owner changes.


## Resize exposes many more tiny lane nodes

The additional resize GJ-wait/v1/v2 packed traces preserve each treatment's
own pilot vector (2,855 entries each), all 101 final sizes, and provenance.
Independent audit covers all nine Hard/resize traces and 24,741 entries;
resize has 64 nonempty/15 empty report iterations and 67 auxiliary invocations,
with no auxiliary builds. Its cross-GJ match differences remain explicit.

For the exact-count Ramp/Ramp rule, GJ constructs just 59 Bop operand nodes
covering 743 rows; fusion removes those. Ramp[2] lane nodes instead rise from
95/340 input rows to 2,037/5,129: 1,084 catalog children and 953 packed children,
all shared dynamic table-family nodes. Total rule builds rise 204 -> 2,039
while input rows fall 45,730 -> 12,972. By comparison, Hard32 lane builds rise
97 -> 435 (294 -> 982 rows), and Hard128 258 -> 987 (720 -> 2,110 rows).
The untraced resize repeat has 22,136 matches in every Ramp/Ramp treatment,
and mean time 4.766 -> 5.569/5.560 ms, all three native pairs slower. This
supports a tradeoff between avoiding larger operand work and creating many
more tiny lane nodes; the trace does not measure repeated probe costs.

Do not broadly undo the tiny-source cache rule: commit b7f2af443 explicitly
measured 40-70 repeat probes per published source on Whisper, with repeated
stack construction costing about a fifth of search time. No archived
singleton-only bypass was found. The next bounded experiment will compare a
one-row SmallColumn fallback with and without preserving a validated original
shared source; >1-row behavior stays as-is. Preservation must exclude local
Direct slots, whose family-insensitive cache could otherwise return a node for
the skipped column. Shared table-family publication distinguishes successors.
This is a planned diagnostic, not a retained production change.


## Singleton-only packed bypass loses in every panel

The temporary singleton patch passes formatting and all 177 core tests in five
configurations: off, GJ/PK inline, and GJ/PK preserve. Its three focused tests
cover scalar lookup, sorted intersection, full/ranged enumeration, stale rows,
slow constraints, and downstream family independence. Preservation is limited
to shared Dynamic family slots; local Direct sources fall back to inline rows.
The frozen binary is a189249e83d06c16263cbfca169c4305617f61d22011b1aebf5ac13733ca6900.
All temporary source is restored, including the original user patch.

The screen uses separate matched GJ and PK-v1 panels on Hard32, Hard128, resize,
and Whisper. Each panel compares off/inline/preserve in the same binary with
three timed runs and one warmup, CPU24/j1/no-decomp. All 96 runs and 342,888
ordered entries pass independent provenance, command, environment, raw match,
changed-flag, aggregate, updated/can_stop, and stdout-size checks within each
panel. The documented GJ/native resize count differences remain explicit;
they are not attributed to the singleton experiment.

Mean query changes versus off (inline / preserve):

| Workload | GJ panel | PK-v1 panel |
| --- | --- | --- |
| Hard32 | +6.17% / +7.09% | +6.21% / +5.71% |
| Hard128 | +11.72% / +11.75% | +10.29% / +9.77% |
| resize | +2.55% / +2.37% | +2.82% / +2.94% |
| Whisper | +27.06% / +26.93% | +25.74% / +24.74% |

Both candidates lose all 48 paired query comparisons, in both execution-order
groups. Hard128 wall losses are about 7-9%; Whisper about 13-15%. The three-Ramp
total also gets worse in all panel/mode means. A small Hard128/v1 preserved
divisible-lanes saving (24.7 microseconds, 1.08%, all three pairs) is overwhelmed
by 31.05 ms of whole-query regression. In Whisper the FE/FE Mul and Add merge
rules account for roughly 80 ms of the GJ query loss. Preserving a validated
original source does not rescue bypassing reusable singleton nodes. Reject both
modes without broader confirmation. The experimental hook also reaches ordinary
scalar Intersect calls, so no unrelated probing change is promoted.

Existing traces further weaken singleton construction as the explanation for
resize's original PK regression. Seven Ramp/Ramp entries with exclusively
singleton Ramp[2] builds contain 408 such builds but together improve by about
98.7 microseconds under v2. Two late entries (iterations 68 and 71, ordinal 1)
instead add 78.9/81.2 microseconds while building no new Ramp[2] nodes. This
alignment uses existing untraced timings, not diagnostic-clock estimates. It
points to other execution costs; it does not yet identify the expensive probe
or scan. Artifacts: pk-singleton-panels/independent-singleton-audit.json/.txt,
and packed-build-resize/singleton-repeat-attribution.json.

## Why repeating the current PK planner is insufficient

PK-domain eligibility, grouping, and its unary-domain anchor are structural;
table size only chooses among eligible covers. Each measured Ramp rule has
one eligible Bop cover, so refreshing statistics alone rebuilds the same shape.
The current seminaive path refreshes headers while retaining cached stages.
A useful GJ/PK runtime choice would need a measured admission policy and two
prepared variants, including their shared access-family registration.

Final Bop/Ramp row ratios are 0.896/0.903 for Hard32/128 and 0.834 for resize;
AddOrSub has two rows everywhere. These endpoint statistics favor resize even
more despite its regression, and cannot explain residual reuse. They are not
per-entry source sizes. Do not fit an absolute-row threshold to these cases.
The next bounded observation reuses the existing DVO trace design for the exact
Ramp/Ramp rule: live subset sizes, source kinds, physical order, and logical vs
physical refinement credits, with unchanged production cadence and no index
lookup or warming. This can reveal whether a small seminaive input is already
available before the fused cover scan. Source/archive assessment:
pk-runtime-selection-assessment.txt. No new planner policy is retained.


## Late selective deltas expose the missing operand access path

The bounded Ramp/Ramp DVO diagnostic passes formatting and all 174 core tests
both disabled and enabled with PK-v2. Its frozen binary is
e518136563b12c2f3e18531ca3820e64fc4c8fab2374e1060ca3f91fd5c9d0a0.
The patch adds no policy, cache lookup, index warming, or per-row timing;
snapshot printing is inside the query timer, so these durations are diagnostic
only. Temporary sources are restored exactly. Nine GJ-wait/v1/v2 reports on
Hard32, Hard128, and resize preserve each treatment's own 24,741 ordered-entry
oracle, final sizes, aggregates, changed flags, updated/can_stop, and provenance.
The full catalog completion vectors align the scopes to report iterations.
All 740 sorter calls are logged, with no entry reaching the 64-call cap.

The two expensive late resize entries demonstrate a missing access path:

- Iteration 68, ordinal 1 has one match. Initial physical roots are left Ramp
  4,198, right Ramp 1, Bop 3,267, and AddOrSub 2 in all treatments. PK already
  runs the lane intersection before the operator and fused Bop stage. At that
  fused stage the Bop candidate subsets are 2,369 and 33 rows, versus left Ramp
  3,643 and right Ramp 1. GJ's order is lane, right-operand intersection,
  operator, left-operand intersection, with payload scans interspersed.
- Iteration 71, ordinal 1 has six matches. Roots are left Ramp 4,199, right
  Ramp 6, Bop 3,273, AddOrSub 2. PK runs operator then fused Bop, with Bop
  candidate subsets 2,375 and 33. GJ runs operator, right operand, left operand.

Thus the selective right input is available. Grouping removed the scalar
Bop operand intersections that GJ uses to narrow its cover. Moving the lane
intersection earlier cannot restore those accesses; in the one-row case it
is already first. These are cached physical candidate counts, potentially
including stale rows, not measured live scan-row or distinct-key counts.
Absent trace sources mean moved-out state, not empty relations. Complete v1
and v2 target count/order/source vectors are identical across all three inputs.

Hard has the same adverse late shape. At iteration 69, ordinal 1, Hard32's
Bop candidates 549/50 face Ramp 633/2; Hard128's 1,413/62 face Ramp 1,401/2.
Existing untraced five-run means change 11.279 -> 30.090 microseconds at32,
20.872 -> 70.638 at128. This is per-invocation selectivity, not a resize-only
topology. Conversely, Hard128 iteration 61, ordinal 1 has Bop 1,409/62 versus
Ramp 1,987/64 yet PK wins 169.629 -> 61.159 microseconds. Its GJ recursive
ordering chooses the lane and left-operand path before the small right side.
A blanket cover-larger-than-probe rejection would discard that benefit.

Broad size-first execution is not wholly unexplored: commits 38bd3f43 and
fdcc50db survive on other branches, but are not ancestors of this HEAD. They
also alter cadence, prefix handling, or retained-row estimates; their recorded
results do not isolate a narrow fused-credit change against this baseline.
Do not repeat them as if new. A scheduling-only change also cannot restore
the removed operand accesses observed above.

Next assess two fixed decisions offline against existing untraced entries:
only fuse when its root is no larger than both incident roots; alternatively,
fall back only for an incident root within the existing eight-row scalar
small-source path when the cover is larger. These are hypothetical additive
timing estimates, not measured hybrid wins: switching plans changes shared
index reuse, and existing cross-planner match differences remain relevant.
Require evidence of useful potential before adding two-plan runtime machinery.
Artifacts: ramp-dvo-diagnostics/independent-dvo-audit.json/.txt,
concrete-late-records.json/.txt, and ramp-dvo-prior-policy-review.txt.

The two fixed offline gates do not establish a good runtime policy. Selecting
PK only when Bop's root is no larger than either incident Ramp selects 9/34,
10/34, and 9/36 target entries at Hard32/Hard128/resize, mostly the Bop-delta
ordinal. Historical additive target changes versus GJ are -3.84%/-7.55%/+2.67%
(37.8/182.3 microseconds saved, then 127.2 microseconds lost). This drops useful
medium-size Hard work while retaining a losing resize ordinal-2 component.

Using the existing SMALL_RESIDUAL=8 boundary instead, and falling back only
when an incident root is tiny and Bop is larger, retains 29/34, 29/34, and
30/36 PK entries. V2 additive target changes are -11.58%/-16.82%/+11.84%
(114.0/406.1 microseconds saved, 564.1 microseconds lost). It recovers
28.4/72.8/229.8 microseconds relative to always-PK, preserving the useful
medium-size Hard entry, but most resize loss remains. V1 gives nearly the
same result. Both execution-order groups retain these signs. Entries without
a root snapshot explicitly use GJ rather than invented cardinalities.

These estimates combine existing per-entry times; they exclude selection and
dual-preparation overhead and cannot model changed cross-rule index reuse or
representative evolution. No threshold sweep was performed. Do not implement
or promote two-plan machinery from these insufficient estimates. The small
remaining physical question is whether ordering the already-built fused
probers by key count helps the forced PK path's large-left/tiny-right shape;
earlier general runtime screens predate this PK policy. Verify that archive
distinction before repeating a probe-order experiment. Offline artifacts:
offline-root-size-policy.json/.txt and offline-tiny-root-policy-v1/v2.json/.txt
under ramp-dvo-diagnostics.


## Ordering existing fused probers has a repeatable local signal

Archive review found EGGLOG_FUSED_ORDER_PROBES in older bundled runtime
experiments, but no standalone comparison on the conditional PK-domain path.
The new minimal patch stably sorts the already-built descriptors by existing
Prober::len(), retaining their original ordinals. It changes only filtered
FusedIntersect probe order; index construction, cover traversal, binding
writes, rollback, and accepted-frame order remain unchanged. It adds no
empty-index shortcut, deferred writes, or planner change. Later lazy descents
can naturally be avoided when an earlier probe rejects a row.

Formatting and all 174 core tests pass with flags off, ordering on under GJ,
and ordering on under PK. The frozen binary is
ceccc2a1b44712c2067ca74894cd998aaa4b85f70c06717653bf3f480b751d5d.
Temporary source is restored exactly. Separate GJ/v1 panels use three timed
runs and one warmup on Hard32, Hard128, resize, and Whisper, CPU24/j1/no-decomp.
All 64 reports and 228,592 ordered entries pass the full within-panel oracle
and provenance audit. Resize's known cross-planner differences remain explicit.

All 27 native Ramp target comparisons improve (three rules, three workloads,
three timed pairs), with both execution-order groups agreeing. Combined trio
time falls 9.15%/9.36%/10.02% on Hard32/Hard128/resize, saving
0.246/0.624/1.320 ms. Native whole-query changes are -0.58%/-0.62%/-2.40%;
wall changes +0.66%/-0.49%/-0.76%. GJ Hard32's apparent 8.3% query benefit is
dominated by one slow off sample (69.3 ms versus later 53.7/53.8 ms), with
merge, rebuild and wall also elevated; it is not an ordering speedup claim.
Whisper native query rises 0.86% in all three pairs, with the first pair
accounting for much of it; matched GJ is neutral. Repeat before concluding.

Late resize corroborates the predicted failed-probe savings using untraced
samples: iteration 68/ordinal 1 falls 82.692 -> 37.210 microseconds, and
71/ordinal 1 falls 88.321 -> 41.527, all three pairs. Their ordinal-0 mirrors,
where the small left input is already probed first, change only about one
microsecond. The two highlighted entries contribute 92.277 microseconds of
the Ramp/Ramp target's 352.055-microsecond saving (5.615684 -> 5.263629 ms,
6.27%, all pairs and both order groups). GJ still takes roughly 2.03/4.50
microseconds in those late entries. Ordering reduces failed-path work but
cannot recover the scalar operand access path removed by fusion.

This screen merits confirmation, not promotion. Next interleave matched GJ,
PK-off and PK-on for five timed runs plus two warmups on these four inputs,
then assess native MinCover/PureSize behavior if the signal holds. Artifacts:
pk-probe-order-panels/independent-probe-order-audit.json/.txt and
ramp-dvo-diagnostics/runtime-interpretation.txt. No production sort is retained
yet, and traced runtimes are not used in any performance claim above.

The interleaved confirmation completes 84 runs (five timed, two warmups),
covering 300,027 ordered entries. Every label matches its own prior vector
and sizes; PK-off/on agree exactly. Resize's driver exit 2 reflects only the
already documented GJ/PK differences, independently audited rather than
ignored. All 45 native Ramp comparisons improve in both order groups. Trio
time falls 8.30%/7.39%/9.66% at Hard32/Hard128/resize. The two late resize
ordinal-1 entries improve 52.80%/52.38% in all five pairs, remaining
38.53/41.45 microseconds versus GJ 2.08/4.64 microseconds.

PK-on/off whole-query and wall changes are Hard32 -0.11%/-0.19%, Hard128
+0.52%/+0.45%, resize -2.29%/-0.65%, Whisper -0.93%/-0.32%. The local win
does not imply a general process win. Hard128's +1.594 ms query change is
the trio's -0.487 ms plus other rules' +2.081 ms; ConvolutionShuffle+ and two
WMMA GEMM rules account for 86.8% of the latter. Non-Ramp paired deltas are
-0.262/+3.975/+5.026/+0.571/+1.094 ms. The two large observations occur in
opposite order groups; do not assign that outside-target shift to sorting
without additional evidence.

Default no-decomp GJ cannot reach this sort: plan_gj supplies singleton groups,
compile_stage emits Intersect for their nonempty filters, and scan merging
only merges empty-probe stages. The bridge's native table rebuilder also
bypasses the internal MinCover rebuild-query fallback. Saved GJ plan census
covers 24,617 reported plans and 96,050 scan-only fused stages with zero
filtered fused stages. Thus GJ timing shifts are controls for variability,
not ordering gains; Whisper's zero-admission PK shape similarly makes its
reversed screen/confirmation sign a control observation. Actual changed-path
coverage comes from PK, native strategies and explicit core tests.

The next validation is bounded PureSize/live-MinCover ordering off/on across
known completing shapes, retaining the current filtered-stage cadence and
avoiding known full-native timeout cases. No production change is retained
pending that check. Confirmation artifacts: pk-probe-order-confirm/
independent-probe-order-confirm-audit.json/.txt and
hard128-unchanged-rule-attribution.json. The screen's directional attribution
is in pk-probe-order-panels/runtime-entry-analysis.json/.txt.


## Native validation exposes a mixed-probe ordering regression

The bounded native check completes 190/190 engine runs: seven inputs, nineteen
GJ/PureSize/live-MinCover panels, ordering off/on, three timed runs plus two
warmups. A separate hardened audit verifies all 654,790 ordered entries,
unique chronological coverage, commands/environments, CPU24/j1/no-decomp,
nonempty raw stdout sizes, raw timing/row consistency, input suffixes, binary,
patch stack, and harness provenance. All nineteen within-strategy panels are
complete and exact. Hard32 and eggcc driver exit 2 comes solely from separate
cross-strategy fingerprints; it is not an ordering-induced snapshot mismatch.

Native ordering helps Llama/Qwen Pure query by 1.65%/1.32% and live MinCover
by 2.30%/2.65%, every timed pair. Pure's two cublas column-major rules improve
about 3-4%; live MinCover's pair-fuse lhs Mul/Add rules improve about 21% on
both models. Exact RMS Pure target improves 15.69%, saving 0.887 ms. However,
eggcc live MinCover query rises 3.945% and wall 3.124%, all three pairs and
both order groups. Pure eggcc instead improves 2.871% query. General sorting
is therefore not promoted. The exact Whisper beta family also has no local
gain: Pure adds 7.8 microseconds and live MinCover 19.2 microseconds; the
latter loses all three pairs despite a small whole-query control decrease.

The recursive ExtractedVecOperandHelper rule explains 97.4% of eggcc live
MinCover's query loss: 460.448 -> 484.518 ms (+24.070 ms, 5.227%), exact 537
matches across 358 entries. Its 238 zero-output entries add 22.575 ms, or
93.8% of the rule's penalty; the largest losses include iterations 95-101,
ordinal 2. The same rule under Pure improves 51.146 -> 44.920 ms (12.173%),
including 5.861 ms saved on zero-output entries. This is a plan-dependent
cost difference, not the user's known nondeterministic eggcc snapshot issue.

Two untimed full-native with-plan captures preserve each strategy's own
counts, changed flags, aggregates, updated/can_stop and sizes. They retain
the surrounding native workload rather than restricting the strategy to
the target rule. Each strategy has one target logical plan across all 358
entries. The important Min stage scans VecOperand-get(vec,index,operand,...)
and originally probes ExtractedOperand[operand], then
ExtractedVecOperandHelper[vec,index], then VO[vec]: widths [1,2,1]. Pure instead
scans ExtractedVecOperandHelper and probes VecOperand-get[vec,index], then
VO[vec]: widths [2,1], leaving operand extraction as later stages. Both share
an initial VecOperandAndCost stage with two scalar probes.

The length accessor used by the experiment is heterogeneous: catalog tuple
indexes count full keys, packed tuples count only first-level keys, and
SmallExact counts retained rows. Packed tuple probes may also lazily descend
after a prefix hit. This could rank an expensive tuple probe before a cheaper
rejecting scalar probe. Static widths establish that the suspect combination
exists, but do not establish its actual runtime representation or prove the
cause of the measured loss. Do not claim avoided probe counts from these plans.

Next test one small alternative: stable scalar-first ordering, then existing
key-count order within scalar and tuple groups. It preserves the successful
all-scalar PK order exactly and requires no new statistics or cache lookups.
Compare off/cardinality/scalar-first within each existing native strategy;
no threshold sweep or production promotion is justified yet. The flag-free
cardinality-sort artifact remains unapplied. A retained policy will also need
a focused relational oracle for reordered probes with different projections
and partial failures, which existing production tests do not force directly.

Artifacts: native-probe-order-confirm/independent-hardened-audit.json/.txt,
journal-ready-independent.txt, native-rule-interpretation.json/.txt,
eggcc-helper-zero-entry-attribution.json; eggcc-probe-order-plans/manifest.json
and probe-width-analysis.json/.txt. The unchanged production workspace still
contains only the original user's source edits.

## Scalar-first ordering repairs one regression but gives back other gains

The bounded three-mode comparison completes all 180 runs: seven native cases,
twelve strategy panels, original/length/scalar-first ordering, three timed runs
plus two warmups. The independent audit accepts all 635,490 ordered entries,
all within-strategy fingerprints and prior own-strategy references, and the
complete command/environment/input/binary/patch provenance. All seven drivers
exit zero. Runs use CPU24, j1, --no-decomp, time-only reports, 5-second watchdog
and 32 GiB guard. No cross-binary timing comparisons enter the conclusions.

The frozen binary is egglog-probe-order-policy, SHA 9d98cd685705e333e4012eaede02e64a1994835eef5ad40fc11435e96c1b2e64.
Policy be0cb15d replaces the previous length-only flag patch after the same
eight-base stack. Test-only patch 94dab1a3 adds an independent three-probe
relational oracle: reversed tuple/cover projections, three descriptor rotations,
partial failures followed by success, stale and slow-constraint rejection,
payload fanout, and existence-only versus retained-row probes at one/four
workers. Fresh access families follow each forced stage order. All 175 core
tests pass in five configurations (off, length/scalar with GJ and PK); formatting
passes and the exact original source diff is restored.

Scalar-first improves eggcc live-MinCover query/wall by 3.004%/2.361% relative
to length order, all three pairs and both execution-order groups. The helper
rule falls 496.540 -> 477.986 ms, saving 18.554 ms (3.737%); its 238 zero-output
entries save 19.552 ms while 120 positive entries add 0.998 ms. All iterations
95-101/ordinal2 improve in all three pairs. Against original ordering, however,
the helper gains only 1.031% with two of three pairs faster; whole query is
-0.928%. This repairs the length-order loss, not a confirmed large new win.

PureSize exhibits the opposite tradeoff. Eggcc's helper rises 44.839 ->
48.485 ms with scalar-first, +8.133% relative to length order, all three pairs
and both orders; zero-output entries account for 94.8% of that loss. It still
beats original order by 6.307%. Llama/Qwen Pure query loses 1.257%/1.588%
relative to length order, every pair. Qwen Pure also loses to original order:
+0.823% query and +0.964% wall, all three pairs and both order groups. Requested
width alone is therefore insufficient; no general sorting policy is promoted.

Other local benefits remain: Llama/Qwen live-MinCover scalar query improves
1.843%/2.265% against original, every pair. RMS Pure retains its target gain
(-14.735%, all three), essentially matching length order. Whisper beta has no
target improvement; its whole-query movements cannot be attributed to the
small beta target. CYK live-MinCover scalar wall rises 1.213% in every pair,
while query is only +0.340% with mixed signs. These are native-plan results,
not evidence that default GJ now reaches filtered FusedIntersect.

Next bounded experiment starts from length order and, after a failed probe
and rollback, moves that descriptor to the front for the next cover row.
It uses only observed rejection, local to one invocation, without counters,
clocks, thresholds, extra probes or replanning. The archive review found no
prior controlled failure-driven filtered-Fused ordering attempt; earlier
"adaptive probing" meant driver reversal. Risks include descriptor movement
and thrashing when rejecting predicates alternate. Keep original key/prepared
ordinals, cover traversal and action order. This is a new experiment, not a
retained policy or proof that packed first-level cardinality caused the loss.

Artifacts: native-probe-order-policy/independent-policy-audit.json/.txt,
journal-ready-independent.txt, native-policy-interpretation.json/.txt;
probe-order-fixture/three-probe-tests.patch;
probe-order-policy/adaptive-followup-assessment.txt. All experiment source
remains temporary and the user source diff is unchanged.
