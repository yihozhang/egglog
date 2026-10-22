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
