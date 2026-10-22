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

## Failure-driven move-to-front ordering is rejected

The feedback experiment starts from stable length order and, after a failed
probe and frame rollback, moves that descriptor to the front for the next cover
row. State is local to one invocation; there are no counters, clocks, thresholds,
extra lookups or replanning. Fixed controls retain their original iterator loop
behind one mode branch per cover row. This temporary implementation changes only
filtered FusedIntersect. All 175 core tests pass in seven configurations, including
the same three-probe oracle with adaptive GJ/PK; formatting passes and source is
restored. Frozen binary egglog-probe-order-feedback SHA 6fc91e9f66cf185c69d3b8359da26a529799280d41e5780669d5ea62bdd6f2ca;
policy 2e3486f4, fixture 94dab1a3, builder a045423b.

All 240 native runs pass the hardened audit: twelve panels, four modes, three
timed runs plus two warmups, 847,320 exact ordered entries, all prior own-mode
and within-strategy fingerprints, and complete provenance. CPU24/j1/no-decomp,
5-second watchdog and 32 GiB guard remain unchanged. All seven driver exits
are zero. Comparisons use this one binary, not historical timing baselines.

Performance decisively rejects feedback ordering. Eggcc live-MinCover query
rises 14.932% against original, 12.278% against length and 15.501% against
scalar-first, all three pairs and both execution orders. The helper costs
470.634/486.492/468.247/565.074 ms for off/length/scalar/adaptive. Relative to
length, its 238 zero-output entries add 54.752 ms (+13.856%) and 120 positive
entries add 23.830 ms (+26.086%); both splits lose every pair. The helper is
20.067% slower than original. This is not an eggcc snapshot discrepancy.

Llama/Qwen adaptive query also loses to length order in every native pair:
Pure +1.734%/+1.688%, live-MinCover +5.981%/+6.455%. The latter's sigmoid and
expert marker rules add 19.709/29.137 ms. Hard32 Pure and both CYK panels lose
roughly 2% against original. RMS still beats original, but its target loses
1.504% against length in every pair. Eggcc Pure's helper remains 11.891%
faster than original yet loses 1.947% against length. No confirmation or
production promotion is warranted. These measurements establish a bad policy;
they do not isolate descriptor movement, rejection-order thrashing, or lazy
index effects as its cause.

Next test a narrower static policy: sort by length only when every requested
probe is scalar, leaving mixed-width stages in their planned order. This keeps
the successful all-scalar PK ordering exactly and avoids directly comparing
heterogeneous tuple/scalar key counts. It adds no threshold or statistics.
Use the original single-traversal policy base, removing the failed adaptive
loop and macro entirely. The existing oracle remains sufficient.

Artifacts: native-probe-order-feedback/independent-feedback-audit.json/.txt,
journal-ready-independent.txt, native-feedback-interpretation.json/.txt;
probe-order-feedback/feedback-policy.patch. All experiment code remains
temporary; the production workspace retains only the original user edits.

## Scalar-only ordering passes the discovery screen, pending confirmation

The next policy sorts by Prober.len only when every requested probe has width
one. Mixed-width stages retain their original order. Construction and original
key/prepared ordinals are unchanged, and the execution loop plus all following
source is byte-identical to the base: no feedback macro, duplicate traversal or
per-row mode branch remains. All 175 core tests pass in seven configurations;
formatting passes, and the exact original source diff is restored. Frozen
egglog-probe-order-scalar-only SHA f3ee3b7ae8e681793850da867777ad99732d4184770e2ba0c318f69fd06030e1;
policy 16e3693d, builder eb08f559, unchanged fixture 94dab1a3.

All 240 runs and 847,320 ordered entries pass the hardened audit, twelve
complete native panels with zero exclusions. All own-prior and within-strategy
counts, changed flags and size fingerprints are exact. Same CPU24/j1/no-decomp,
three timed/two warmup runs, resource limits and provenance checks apply. Only
same-binary comparisons are used.

Against original ordering, Llama/Qwen live-MinCover query improves
2.500%/2.551% and wall 1.925%/2.036%, all three pairs and both execution orders.
The Min GEMM pair improves about 25% in both models, binary pair-fuse about
19-21%, and batch-collapse about 15-19%. Pure GEMM gains are smaller (2.812%/
1.716%), as are Pure whole-query changes (Llama -0.806%, all three; Qwen
-0.480%, two of three). RMS Pure target falls 5.623 -> 4.799 ms (-14.655%),
all three pairs, essentially retaining length ordering's benefit.

Eggcc live-MinCover helper costs 462.858/488.115/461.417/459.139 ms for
off/length/scalar-first/scalar-only. Scalar-only removes length ordering's
regression: -28.976 ms (-5.936%), all three pairs and both orders. Zero-output
entries save 24.928 ms and positive entries 4.048 ms. Against original, helper
and whole-query gains are small (-0.804%/-0.415%), although all three pairs
improve. Against scalar-first, the helper mean improves 0.494% but only one
of three pairs; do not present that comparison as established.

Pure eggcc gives back length ordering's helper benefit: scalar-only 50.699 ms
versus length 44.911 ms, +12.888%, every pair; it barely beats original
51.335 ms (-1.238%). The apparent whole-query -6.529% against original is
dominated by one off run's Gamma/ival rule (196.486 ms versus 151.433/152.312
in the other off runs). Whole-query pair changes are -0.843/-16.584/-0.111%,
median -0.843%; this is not evidence of a repeatable 6.5% gain.

There is no clear original-relative regression in this screen. Hard32 Pure
and CYK Min mean query changes are only +0.219%/+0.131% with mixed signs.
Whisper beta targets remain slightly slower (+1.039%/+1.418%) with mixed pairs.
This is discovery evidence, not promotion: confirm original versus scalar-only
with five timed pairs on the native panels, then recheck the all-scalar PK
Ramp panels with the same frozen binary. Default GJ still does not reach this
filtered stage policy. Separately revisit saved MinCover/PureSize wins against
GJ to find a planning opportunity; six zero-output loop-unroll rules are a
small outstanding candidate, not yet a selectively measured win.

Artifacts: native-probe-order-scalar-only/independent-scalar-only-audit.json/.txt,
journal-ready-independent.txt, native-scalar-only-interpretation.json/.txt;
probe-order-scalar-only/scalar-only-policy.patch. No experimental production
code is retained yet.

## Five-pair scalar-only confirmation: local wins and one small tradeoff

The same f3ee frozen binary confirms original versus scalar-only on all twelve
native panels with five timed pairs and two warmups: 168 successful runs,
593,124 ordered entries, all exact own-reference and within-strategy fingerprints,
complete provenance, no excluded panels. All seven drivers exit zero. No build
or other engine overlaps measurement; CPU24/j1/no-decomp and prior limits apply.

Query/wall changes are Llama live-MinCover -2.985%/-2.020%, Qwen Pure
-2.016%/-1.351%, Qwen live-MinCover -1.575%/-1.152%, and eggcc live-MinCover
-1.483%/-1.105%. Each improves all five pairs and both order groups. RMS Pure
target improves 14.386% in all five pairs; whole query improves 7.861%.
Llama Pure is only -0.493% query, four of five pairs. Hard32 and CYK are near
neutral with mixed pairs.

The eggcc Pure discovery outlier does not recur. Confirmation instead finds
+0.647% query (four of five slower, median +0.769%), with the Gamma/ival rule
adding 1.460 ms against 1.407 ms total query growth. Wall is +0.028% with mixed
pairs, including one slow off wall observation. Whisper beta also has small
target losses: Pure +14.9 microseconds/+1.144% (four of five), live-MinCover
+18.53 microseconds/+2.507% (all five and both orders); whole query is neutral.
Retain these tradeoffs rather than presenting the policy as uniformly faster.

A separate same-binary PK confirmation completes 84 runs and 300,027 ordered
entries, each exact to its own prior planner reference; PK off/scalar-only
vectors and final sizes are identical. All 45 individual Ramp target pairs
improve, in both execution orders. Three-rule sums improve Hard32 9.088%
(2.700 -> 2.455 ms), Hard128 7.884% (6.648 -> 6.124 ms), and resize 8.886%
(13.162 -> 11.993 ms). Whole-query/wall changes against PK-off are
-0.621%/-0.324%, -1.184%/-0.584%, and -1.594%/-0.443%; query improves all five.
Hard128's 3.642 ms query saving exceeds the target's 0.524 ms, so do not assign
all surrounding-rule movement to this sort. Whisper is a zero-admission control
with mixed timing shifts, not a sorting result.

The PK planner itself remains unpromoted. Resize scalar-only still loses to
GJ by 0.935% query in all five pairs, and Ramp/Ramp remains 9.559% slower.
Late entries 68/ordinal1 and 71/ordinal1 improve 57.469%/51.909% against PK-off,
but cost 37.756/41.661 microseconds versus GJ's 2.059/4.563. The known sixteen
cross-GJ ordered-entry differences remain explicit; within-PK equality is exact,
and no blanket cross-planner equivalence is inferred. Resize driver exit2 is
that known distinction, not a new ordering failure.

Two untimed full-Pure Gamma captures with the current binary preserve all
own-mode count/changed/aggregate/updated/can_stop/final-size oracles. Independent
audit accepts 10,502 full ordered entries and fourteen target entries. Its seven
entries and nine matches have the same logical plan: Gamma cover with scalar
VVO[col1] and Project[col1] probes, followed by scan-only stages. Older saved
Pure/live-MinCover plans already show that shape and match current entry counts.
The fresh captures do not establish actual runtime representation, lengths,
probe order or rejection rates. The same Gamma target under live-MinCover has
a small mixed-sign improvement, so a new width/length threshold is unwarranted.
Hold the flag-free candidate while assessing that runtime question and finishing
the persistent scalar-plus-tuple oracle; broad production validation is still
required before retaining code.

Saved native-versus-GJ mining provides a separate planning lead: six trips=4
Add/Mul/LessThan loop-unroll rules under live-MinCover save 0.802/0.818 ms
on Llama/Qwen (about 46-47%), every individual rule and all three saved pairs.
All 58/59 per-rule ordered entries are exact but produce zero matches, and the
native workload context differs. Pure loses on this family. Prior notes flagged
it from one sample, but no isolated confirmation was found. Exact selectors and
source paths are prepared; a selective experiment must establish the benefit
before deriving a structural planner rule. Other unexplored positive-output
native wins are at most 59 microseconds.

Artifacts: native-scalar-only-confirm/independent-confirm-audit.json/.txt
and journal-ready-independent.txt; pk-scalar-only-confirm/
independent-pk-scalar-only-audit.json/.txt; eggcc-gamma-order-plans/manifest.json;
eggcc-gamma-order-plans/independent-plan-audit.json/.txt;
native-rule-win-reassessment/README.txt, analysis.json and selectors.json;
scalar-only-production/scalar-only.patch (unapplied). The workspace still
contains only the original user source edits.

## Flag-free scalar-only candidate checks

The production candidate contains only the mutable descriptor vector and the
all-scalar stable length sort; it has no experimental planner/ordering flags,
feedback traversal or diagnostic code. Its separate persistent fixture v2 uses
real T(q,x) and T(q,y,x) schemas, with independent 13/21 versus 9/17 output
oracles, one/four workers and all descriptor rotations. This exercises actual
scalar reordering and mixed-stage fallback without relying on temporary PK
tests. All 167 production core tests and 19 bridge tests pass, as does formatting.
Both release binaries are frozen and the exact original source diff restored.

Control SHA bff505ff3b2047f8b72169c73c6cfe49241184865abcba9d38ef182508a0f71d;
candidate SHA 82e5e49d4a63f7865a7263c8f56e14f85e3fe41926329d74f09d3cdeff476784.
Metadata: scalar-only-production-build/manifest.json, test-core.log and
test-bridge.log. Source patches are scalar-only-production/scalar-only.patch
(65aae112) and probe-order-fixture-v2/three-probe-tests-v2.patch (4e024928).
Neither is retained yet. Broad production validation and a bounded untimed
Gamma runtime-order diagnostic are being prepared; no further static capture
or size-threshold fitting is planned.

## Scalar-only production screen: neutral aggregate, Hard128 follow-up required

The frozen flag-free control/candidate complete all root11 wall runs: fifteen
timed pairs and three requested warmups, 330 successful recorded timed processes,
original unchanged inputs, CPU24/j1/no-decomp, no overlapping builds or engines.
Summed mean wall is 1.993670 -> 1.993514 seconds (-0.0078%); seven of eleven
case means improve. The material loss is Hard128: 338.434 -> 341.845 ms
(+1.008%), paired median +0.950%, fourteen of fifteen pairs slower. Both order
groups agree (+1.090% control-first, +0.915% candidate-first). Hard32 is +0.309%;
all other mean losses are below 0.18%. Aggregate neutrality does not resolve
the Hard128 signal; a focused repeat and generated-code inspection are pending.

Separate untimed root11 oracles accept all 22 reports and 140,236 ordered
per-rule entries, including counts, changed/updated/can_stop, aggregate counts
and nonempty final table-size maps. These inputs add only print-size/print-stats.
No eggcc nondeterminism exception is needed. This establishes the recorded
logical/count/size oracles, not equality of every table's contents.

Math at iter12 completes three timed pairs plus one warmup: all eight reports
and 2,816 ordered entries are exact. Query is +0.240% in all three pairs, while
wall is -0.069% with mixed pairs. Merge +1.223% and rebuild -0.715% largely
cancel in total time. The scalar sort is logically inactive in default no-decomp
GJ, so these changes are regression signals or variability, not different probe
work. No causal compiler/layout conclusion has been established. External81
remains prepared but unrun, and the candidate remains unapplied/on hold.

Independent evidence: scalar-only-production-root11-independent.json/.txt,
scalar-only-production-root11-oracles-independent.json/.txt,
scalar-only-production-math12-independent.json/.txt and
scalar-only-production-validation-independent.txt under /tmp/fused-lab.

## Hard128 repeat and shared-sort code-size hypothesis

A second fifteen-pair original-input run starts with the candidate, balancing
the combined thirty pairs to fifteen per execution order. It confirms +0.913%
mean wall, +1.020% paired median, thirteen of fifteen slower. Across both batches,
with every sample retained, the loss is +0.961% mean, +0.985% paired median,
twenty-seven of thirty slower. Both combined order groups lose (+1.161%/+0.760%).
Full frozen binary/input/helper/environment provenance passes independent audit.
The existing exact root oracle applies to the same binaries. External validation
stays pending while this default-GJ regression is unresolved.

Read-only nm/objdump inspection matches all twelve run_plan monomorphizations by
their complete symbols. Entry stack reservations are unchanged in five, +16 bytes
in four and smaller in three; no broad stack growth is found. Their combined text
grows only 900 bytes. The binary's text instead grows 115,760 bytes, of which
114,257 bytes are 53 additional physical stable-sort bodies: twelve wrappers,
twelve driftsort_main, twelve drift::sort and twelve quicksort bodies, plus five
shared helpers. Each generic run_plan calls its own sort wrapper. This establishes
code duplication, not a causal explanation of the Hard128 timing change.

A small follow-up will put the descriptor sort in one private, concrete-slice
helper with lifetime parameters only, retaining the all-scalar guard and row loop.
The intent is to share the comparator's sort instantiation without forced inline
attributes, padding or a new sorting algorithm. Its code size and performance
still need measurement; no change is retained yet.

Artifacts: scalar-only-production-hard128-repeat-independent.json/.txt;
scalar-only-code-layout/evidence-note.txt, evidence.json, sort-call-evidence.json
and per-symbol disassemblies. All paths are under /tmp/fused-lab.

## Gamma runtime counters: no extra probe work from scalar ordering

A bounded temporary diagnostic observes only the exact Gamma/ival rule's
two-scalar-probe stage, with fixed local counters and no new lookups, clocks or
policy changes. All 175 core tests pass in three configurations and the exact
source/index/status are restored. Two untimed full-Pure runs (off/scalar-only)
preserve 10,502 ordered entries, fourteen target entries, aggregate counts,
changed/updated/can_stop and final sizes; every counter invariant passes.
Instrumented timing is excluded from performance evidence.

The stage executes six times, all in iteration109, and visits 44 cover rows in
both modes. It completes 24 cover frames, which are not final rule matches.
VVO is SmallColumn and Project is PackedColumn; both use Owned sources in all
six acquisitions. Original length pairs are (4,8), (7,8), then four (7,4).
Scalar-only keeps the first two orders and reverses the last four, with no ties.
At ordinal2, Project rejects all four rows and avoids four VVO lookups. The
other outcomes agree: attempts fall 72 -> 68, failures remain twenty, and no
extra cover rows or completed frames appear. The small uninstrumented Gamma
loss therefore is not explained by additional probe work. Effects elsewhere
in the rule or surrounding workload remain unmeasured; no length threshold
or special-case policy follows from this diagnostic.

Artifacts: gamma-probe-runtime-trace/{README.txt,runner-manifest.json,build.py,
collect.py,audit.py}; gamma-probe-runtime-build/manifest.json;
gamma-probe-runtime-diagnostics/audit.json. Diagnostic binary SHA
8f56ae507f642107062ba8f098e427227731cf35cb0947247762eaa86750aa5c.

The separate shared-sort production replacement also passes all 167 core and
19 bridge tests plus formatting and restores the source exactly. It uses one
lifetime-only helper and no inline attributes; the eligibility check and row
loop match the earlier scalar-only candidate. Binary SHA
8c3b76399315ed212ec0d7bbad57ef9d02d1b9b500432257a1d78248eef24605;
scalar-only-shared-sort-build/manifest.json. Generated-code inspection and the
focused Hard128 comparison are pending; it remains unapplied.

## Selective loop-unroll plans captured

Four untimed captures use the original-order ceccc2a1 binary on complete Llama
and Qwen workloads, selecting only both Add/trips=4 orientations for live-MinCover.
Independent audit accepts 15,600 full ordered entries and 468 target entries,
all exact to prior GJ counts/flags/final sizes. Only the 116/118 selected entries
change plans; every other plan stays identical. GJ's 23 stages become ten/nine
native stages. The leading LoopStart cover probes MNum, LoopInput's two-column
key, ICons, and LoopEnd's three-column key. This establishes the selected plan
shape, not its runtime ordering or performance. Both target rules still produce
zero matches. A five-pair selective timing comparison is being prepared.

Artifacts: selected-unroll-plans/independent-plan-audit.json/.txt,
comparison.json and compact-plans.txt; collect-selected-unroll-plans.py.

## Shared-sort code size and focused production check

The lifetime-only helper yields one sort instantiation as intended: all twelve
run_plan variants call the same 96-byte helper, followed by one driftsort_main,
one drift::sort and one quicksort body. Text shrinks 94,160 bytes versus the
inline-sort candidate (7,554,402 -> 7,460,242), leaving +21,600 bytes against
control. Each run_plan body and entry stack reservation is byte-size-identical
to the inline candidate. This is code sharing, not a stack-frame repair, and
does not establish the cause of timing changes.

The focused unchanged-input Hard128 comparison passes provenance checks for
all thirty timed processes. Fifteen pairs give +0.404% mean wall, +0.445% paired
median, ten of fifteen slower. Order groups are +0.366%/+0.448%, both below
0.5%. The prior approximately 1% loss is not reproduced, but no speedup is
claimed. With this bounded follow-up complete, the shared candidate proceeds
to root11, separate exact report oracles and math12; external81 remains pending
those results. The candidate remains unapplied.

Artifacts: scalar-only-shared-sort-code-layout/evidence-note.txt and underlying
symbol/prologue evidence; scalar-only-shared-sort-hard128-independent.json/.txt.

## Isolated MinCover Add/trips4 win confirmed, whole workload neutral

Five timed pairs plus two warmups per model use one frozen ceccc2a1 binary,
CPU24/j1/no-decomp, with only the two selected rules switching GJ to live-MinCover.
All 28 reports, 109,200 full ordered entries and 3,276 target entries pass complete
own-mode and cross-mode count/changed/updated/can_stop/final-size and provenance
audits. No timing samples are excluded. This is a zero-output search experiment.

The two-rule sum is Llama 0.605862 -> 0.373962 ms (-38.276%) and Qwen
0.605562 -> 0.388604 ms (-35.827%). Every pair and both order groups improve;
each orientation independently improves in all five pairs. State0 improves
27.42%/24.19%, state1 49.98%/48.39%. Whole query is -0.153%/+0.351% with mixed
pairs and wall +0.422%/+0.124%; these do not establish a whole-process benefit.

Both inputs end with one LoopStart and one LoopEnd; source loop bounds are
31/35 whereas the selected rules require4. MNum4 is present elsewhere, so an
absent constant table alone does not explain rejection. Actual runtime order,
subsets and reject location remain unknown. LoopStart's private RHS key initial
excludes it from the earlier all-shared-PK heuristic. The next identifying
ablation will isolate just its full-live-row fused cover and leave the remaining
variables to GJ, rather than adopting the entire native suffix or a named-rule
production condition.

Artifacts: selected-unroll-timing/independent-timing-audit.json/.txt and raw runs;
selected-unroll-timing-prepared/manifest.json. No production planner change.

## Shared scalar-ordering production candidate rejected on CYK

The full shared-candidate root11 screen completes 330 timed processes with
verified provenance. Summed wall improves 0.109%, while geometric mean is
+0.0117% and arithmetic mean +0.0235%; seven case means improve. These aggregates
hide CYK's 27.4319 -> 28.6671 ms (+4.503%), +4.674% paired median, all fifteen
pairs slower. Both orders lose (+4.754%/+4.215%). Hard128 is +0.680% and Whisper
+0.445%; Luminal, eggcc and Qwen improve 1.518%/1.139%/0.800%. No timing shift
on default no-decomp GJ is attributed to probe work, since the sort is inactive.

A focused CYK repeat reverses the starting order. All fifteen pairs again lose:
29.2019 -> 30.9851 ms (+6.106%), paired median +4.882%, both orders positive.
Combined, with all sixty timed process records retained, thirty pairs show
+5.330% mean and +4.836% paired median. Both balanced fifteen-pair order groups
lose (+4.985%/+5.672%), and every individual pair is slower. The code-size
reduction is real but did not produce an acceptable production regression profile.
There is no established causal explanation beyond the measured binary comparison.

Separate correctness oracles accept all 22 reports and 140,236 ordered entries,
including full counts/changed/updated/can_stop/aggregate counts and nonempty final
table-size maps. No eggcc exception is required. The standalone flag-free scalar
sort, including the shared-helper variant, is rejected for production. Both
patches and their fixtures remain unapplied; the exact original user source diff
is preserved. Shared-candidate math12/external81 are deliberately not run after
this confirmed material regression. Native ordering remains a useful measured
prototype for future combined planner experiments, not a retained change.

Artifacts: scalar-only-shared-sort-root11-independent.json/.txt,
scalar-only-shared-sort-root11-oracles-independent.json/.txt,
scalar-only-shared-sort-cyk-repeat-independent.json/.txt under /tmp/fused-lab.

## PureSize versus GJ: distinguish planner wins from ordering wins

A read-only recheck of the audited same-ceccc2a1, original-order runs confirms
real positive-output PureSize wins: direct-exp Llama 1.2173 -> 0.7309 ms
(-39.95%) and Qwen 1.4075 -> 0.8201 ms (-41.73%), five matches in each;
list-tail 1.1251 -> 0.9503 ms (-15.54%, 5,509 matches) and 1.7015 -> 1.4404 ms
(-15.35%, 8,169 matches). All three pairs and both order groups improve, with
exact target and full-model count/size oracles. These are already-explored local
families; full native Pure query remains about 13.4x/17.2x GJ in these contexts.

The known Hard32 Ramp/Broadcast equal-lanes rule improves 1.1715 -> 0.9729 ms
(-16.95%, 1,815 matches), with exact target vectors but differing full-Hard counts.
WMMA load improves 49.09% but is zero-output. No unexplored positive-output Pure
winner above 42.85 microseconds remains after the known families are excluded.
The earlier 59-microsecond Project/Theta result was live-MinCover, not Pure.

RMS must not be described as a Pure-over-GJ win: its saved target is GJ
2.3005 ms, original-order Pure 5.6547 ms and size-ordered Pure 4.7674 ms.
The 15.69% within-Pure gain still leaves it over twice GJ. Likewise the confirmed
unroll family favors live-MinCover while Pure loses. These distinctions guide
selective planner work; neither native strategy is recommended wholesale.

Evidence: pure-versus-gj-local-evidence/README.txt, analysis.json, rankings.txt
and artifact-hashes.json under /tmp/fused-lab. No new engine runs.

## One full-live cover plus GJ suffix ablation built

The next temporary planner experiment isolates a single explicitly numbered
cover in the selected Add/trips4 rules. It retains exactly shared/RHS cover
variables, including the private RHS key initial, and proves every physical
primary-key column is retained or explicitly constant. Missing/private-unused
keys reject before state mutation. Ordinary stage construction assigns probes
and constraints; the existing GJ suffix skips bound variables while retaining
other RHS fanout and isolated-atom existence. There is no production table-name,
rule-name, size-ratio or arity admission heuristic. Logical prefix placement
does not imply that DVO executes it first.

The existing independent relational oracle now includes this mode at one/four
workers, covering private RHS keys, stale/slow rejection, repeated/constant
keys, missing witnesses, unused/placeholder probe keys and a real T(x,u) RHS
suffix with two-value fanout. Additional assertions check one binding per cover
variable and constraint/existence assignment for every atom; a proof matrix
checks repeated physical-key representatives and rejected missing keys.

Independent source reviews pass. The frozen first-eight PK-v2 plus original
eed762 probe-order switch plus one-cover patch pass all 175 core and 19 bridge
tests and formatting. ORDER_PROBES=0 is used; the rejected standalone scalar
sort is not a dependency. Source/index/status are restored exactly. The same
compiled artifact will compare GJ, one-cover and selected live-MinCover.
Before timing, six untimed model captures must confirm accepted LoopStart
AtomId1, live columns0..5 without timestamp6, exactly one filtered fused cover
for the ablation, no non-target plan changes, and complete count/size oracles.

Patch: one-live-cover/one-live-cover.patch,
SHA ef309dc1133cd84bd0a1ecfce5ee03632a01c3e7d057b1e9f3ae4f2d1e41d46b.
Binary: egglog-one-live-cover,
SHA 41823df2b8fb99911adf0d8d476566927c294e1759fdef34e2c10c4ad4bdd312.
Artifacts: one-live-cover/{manifest.json,README.md,independent-review.txt};
one-live-cover-build/manifest.json. No engine workload run yet for this artifact.

## One-cover plan captures accepted; timing isolated next

All six untimed captures (Llama/Qwen, GJ/one-cover/live-MinCover) now pass.
The first collector stopped after two successful Llama engines because its
trace parser expected one fewer Some wrapper around the table name. The raw
runs and failed manifest were preserved. A separate hash-pinned resume script
reaudited those two runs and collected only the remaining four; no engine or
count failure occurred, and the shape check was not relaxed.

Independent shape analysis confirms 3,827 Llama and 3,973 Qwen ordered entries,
with all counts and changed flags exact. The 3,711/3,855 non-target plans are
unchanged. Each of the two selected rules has 58/59 entries and zero matches.
One-cover replaces five scalar stages and one pure scan with one filtered
LoopStart cover: 23 stages become 18, all 26 live variables bind once, and the
17 remaining stage descriptions equal the original GJ suffix exactly. Native
live-MinCover uses the same first-cover filters but additionally binds timestamp
column 6 and changes the suffix, yielding nine/ten total stages. Thus the next
same-binary timing comparison can distinguish one-cover behavior from the
whole native plan. Physical DVO order remains unobserved.

Artifacts: one-live-cover-plans-resumed/{manifest.json,audit.json,
independent-shape-audit.json}; original captures and failure are retained under
one-live-cover-plans. No performance claim is made from these traced runs.

## Ready cover-index prefilter: metadata diagnostic prepared

A separate runtime lead asks whether the expensive late Ramp fused scans could
reuse an already-published Bop column index to restrict their cover with a tiny
scalar probe. Saved traces show prior same-invocation raw Bop[2] indexing near
the late resize/Hard entries, but equal row counts do not establish root identity
or publication readiness. The exact current prefix may also lack the index.
This is not the earlier full-primary-key reverse lookup or singleton bypass.

A frozen diagnostic retains each actual initial Bop root Arc and observes only
existing current-prefix, exact-root and whole-table catalog slots after normal
probe construction. It performs no key lookup, enumeration, slot allocation,
family registration, index construction or refresh. Unsupported sources and
catalog leaves without a continuation are distinguished from empty slots.
Its fixed summaries join readiness with driver lengths and cover sizes. The
root and independent reviewer have inspected its identity, scope and cache
access logic; build and workload validation are still pending. All traced
elapsed times will be discarded as performance evidence. If neither exact-root
nor catalog indexes are ready, this reuse-only path should be abandoned rather
than paying to build an index.

Artifact: cover-prefilter-readiness/cover-readiness.patch,
SHA d24c8642fe764e9e93bcfe345de2c8c8463d6b9368122c70650af5de68b1150c;
README.txt specifies the schema. Base is first-eight PK-v2 plus original probe
ordering switch with ORDER=0. No production source change is retained.

## One-cover timing: a minority of the native MinCover gain

The same-binary comparison completes all 42 processes (30 timed, 12 warm).
Independent auditing accepts 163,800 full ordered entries and 4,914 selected
entries against both their own capture and GJ, including changed/updated/can_stop,
aggregate counts and final sizes. All hashes, controls and process chronology
pass. CPU24, j1, --no-decomp, ORDER0 and TRACE0 are fixed; a rotating three-label
schedule gives each label every position and both orders for every pair. No
samples are dropped.

Llama target sum is GJ 0.602888 ms, one-cover 0.543176 ms (-9.904%), live-Min
0.408420 ms (-32.256%). Qwen is 0.605165, 0.564576 (-6.707%), and 0.380265 ms
(-37.164%). Both candidate sums improve in all five pairs and both orders.
The isolated first cover captures only 30.71%/18.05% of full native savings.
Its benefit is concentrated in state1 (-21.19%/-17.77%, all five); state0 instead
loses 0.56%/3.47%, four of five pairs slower. Full live-Min improves both
orientations in every pair. All selected match counts are still zero.

Whole-query effects do not establish a general win: one-cover is +0.071%/-0.313%
and wall -0.148%/+0.757%; query order groups are mixed. Full live-Min query is
+0.431%/-0.726% and wall +0.703%/-0.267%. Qwen one-cover's outsized 277.994 ms
wall sample remains included. This is a useful plan-level ablation, not evidence
for a particular rejection mechanism or a production admission rule. The next
diagnostic should observe actual DVO stage visits and rejected probes in both
orientations; first-cover timestamp and native suffix differences remain possible
contributors. No production patch is retained.

Artifacts: one-live-cover-timing/independent-timing-audit.json/.txt,
local-gain-fractions.json, journal-ready-independent.txt and all raw runs;
one-live-cover-timing-prepared/manifest.json.

## Exact-root readiness confirmed for late Ramp covers

The readiness diagnostic passes 174 core tests in each of three configurations
(off, traced GJ, traced PK-v2), formatting, and exact worktree/index/status
restoration. Its six untimed captures pass independent auditing of all 16,494
ordered entries and 628 selected entries, complete own-mode counts/flags/sizes,
trace identities, summary accounting and provenance. Hard32/128 also match GJ
cross-mode; resize retains the previously known cross-strategy count differences
with equal final sizes. No diagnostic elapsed time is used as performance data.

The exact current cover has no published child index in any of 360 sampled
filtered-stage visits. Raw-root reuse is nevertheless available. For physical
Bop column 2, the raw root is ready in 64/118 Hard32, 64/118 Hard128 and 84/124
resize calls; current table catalogs are ready in 24, 22 and 12 calls respectively.
No catalog is observed stale. Missing continuation positions and unsupported
sources remain distinct from unpublished child slots.

Late Ramp/Ramp entries, ordinal 1, each contain two cover calls:

| Case / iteration | Cover row counts | Right probe len() | Exact raw root | Table catalog |
| --- | --- | --- | --- | --- |
| Hard32 / 61 | 545, 50 | 7 | ready | requires initialization/refresh |
| Hard32 / 69 | 549, 50 | 2 | ready | requires initialization/refresh |
| Hard128 / 61 | 1409, 62 | 19 | unallocated | ready/current |
| Hard128 / 69 | 1413, 62 | 2 | ready | requires initialization/refresh |
| resize / 68 | 2369, 33 | 1 | ready | requires initialization/refresh |
| resize / 71 | 2375, 33 | 6 | ready | requires initialization/refresh |

The reported probe length is metadata, not yet a measurement of distinct keys
after every residual filter; cached index lengths can describe a wider source.
All five raw-ready entries use shared packed current covers with unpublished
child families; Hard128/61 has a catalog leaf without a continuation position.
Thus the old current-prefix index absence does not rule out reusing an existing
root index and intersecting its key groups with the residual cover. Readiness
alone does not establish selectivity or a net speedup. A bounded key-group and
membership diagnostic is warranted next, without index construction or a new
execution policy. A generic implementation would also need access to the initial
root after ordinary bindings have been refined; the diagnostic's named-rule Arc
retention is not itself a production design.

Binary: egglog-cover-readiness,
SHA 67b6ec5c6cc557039ea558451c1ba3898660f62f1c6cf1202d714d76624ce1dc.
Runner manifest SHA e4b9e2fa7f66fdffe8b41cddba07bb18042a412af40a06a3d6d641b29bfa7331.
Artifacts: cover-readiness-build/manifest.json; cover-readiness-diagnostics/
{audit.json,independent-final-audit.json} and six raw captures. No production
source change is retained.

## Follow-up diagnostics: unroll execution and bounded index groups

The isolated unroll trace on the accepted one-cover stack is built. All 175 core
tests pass in four configurations (off, traced GJ, traced one-cover, traced native
live-Min), all 19 bridge tests and formatting pass, and the worktree/index/status
are restored byte-for-byte. The trace copies the initial physical order after
ordinary DVO and counts actual stage visits, projected rows, accepted frames and
original-ordinal probe outcomes. It adds no sort, index acquisition, key lookup
or clock; existing traced durations are only report-alignment identities.
The six own-plan captures are prepared next, with no timing claim.

Interpretation must distinguish actual suffix execution from plan setup. DVO
uses a greedy quadratic selection with evolving refinement counts; shrinking
23 GJ stages to 18 one-cover or nine/ten native stages can reduce setup even
if all plans reject early. Grouped LoopStart preserves aggregate refinement
credits but changes their timing, DEFER eligibility and recursive stage cadence.
Native additionally projects timestamp column 6. Counters can establish work
and reachability differences, not attribute all elapsed savings by themselves.

Separately, the next readiness follow-up will enumerate only already-existing
sorted scalar views, with at most eight keys as an observation bound. It will
count raw group sizes and current-cover membership without collecting row sets,
building indexes or changing execution. Physical candidate rows are distinct
from live output rows; probe length metadata alone is insufficient.

A potentially simpler future reuse design is now identified: an already-cached
unconstrained shared root, keyed by (table, HeaderConstraintId0), is a valid
superset of every current cover for that frozen table. Intersecting its existing
empty-slow-constraint column groups with the current cover could avoid retaining
initial root handles per plan. The follow-up will observe its actual presence,
identity and group sizes rather than infer them from equal cardinalities.
Lookup cost and availability remain unmeasured. Any later execution prototype
must also preserve original top-partition boundaries despite cache-publication
races, and preserve row order after combining disjoint key groups.

Unroll patch SHA 30fcf7930164ededeca36186029bd81118c53e9be79d28087b57028c8436146c;
binary egglog-unroll-one-cover-trace SHA
d5d920c207af578593f6d9fa3d7468617fc2fae8f6df9a6e7b131a2353108223.
Artifacts: unroll-one-cover-trace/{manifest.json,README.txt,unroll-trace.patch};
unroll-one-cover-trace-build/manifest.json. No production change is retained.

## Unroll execution: both fused plans reject before any suffix

All six unroll captures pass the collector's full own count/size/static-plan
oracles: 23,400 ordered entries and 702 selected scopes. Independent stage
interpretation finds the same physical behavior for both orientations and both
models. Each has 57 entries with nonempty roots; the remaining one Llama/two
Qwen entries exit during root acquisition.

GJ executes logical stages [2,4,3,0]: loop_id, dtype and slot_idx each accept
one key, then the MNum trip-count intersection rejects. One-cover and native
live-Min execute only logical stage 0 at physical position 0. Each projects
exactly one LoopStart row and fails original probe 0 (MNum). Every subsequent
probe attempt and accepted frame is zero. No suffix stage executes in either
fused plan. Stage-0 input row/source summaries and probe lengths agree between
the two fused strategies; all input roots are shared Owned sources.

This identifies the early-rejection benefit relative to GJ, while ruling out
executed suffix search as the explanation for native's additional advantage.
Setup, plan-size-dependent ordering, timestamp projection and prepared access
metadata remain distinct possible contributors. In particular, the executor
constructs all four fused probers before the first MNum probe rejects, so equal
attempt counts do not establish equal preparation cost. A read-only archive
and implementation review will check whether deferring unused fused prober
construction is an untried modest runtime improvement; no policy is implemented
from this observation yet. Traced elapsed fields are excluded from conclusions.

Artifacts: unroll-one-cover-diagnostics/{audit.json,
independent-stage-interpretation.json,independent-stage-interpretation.txt};
unroll-one-cover-collector-prepared/manifest.json. Independent final provenance
and stronger filtered-counter accounting also pass: all 59 pins, eleven patches,
four 175-test core configurations, 19 bridge tests, formatting and exact saved
source/index/status bytes are verified. All 1,368 stage records are accounted
for; all 456 filtered visits satisfy probe0-attempts = projected rows,
successive-attempts = previous-passes, frames = last-passes, and frames plus
failures = projected rows. No early-empty prober-count exception occurred.
See independent-final-audit.json/.txt in the same diagnostic directory.

## Lazy fused-prober acquisition is an untried runtime lead

Read-only archive review finds no previous lazy original-order fused acquisition
trial; earlier deferred-binding, empty-index, ordering and probe-once experiments
retain eager descriptor collection or address a different mechanism. The current
trace establishes 684 unused acquisitions out of 912 per fused strategy (75%),
1,368/1,824 combined. These are acquisition calls, not measured fresh builds.

A temporary prefix prototype is warranted without another timing diagnostic.
Existing probers can be reused in their original order; only a cover row that
passes the built prefix requests the next descriptor. No successful frame can
drain before all probes exist, so recursive execution sees the original fully
built state. Rejected rows still roll back; only actually acquired sources need
restoration, while untouched sources remain in BindingInfo. A prototype must
verify these invariants with independent relational cases, later successful rows,
draining, stale/slow filters, tuples and one/four workers. It will preserve eager
behavior when the temporary probe-order switch requires all lengths up front.

No lazy policy is implemented or promoted yet. Same-binary measurements must
determine whether skipped work exceeds the additional completion check, including
possible shifts in shared-index construction to later rules. A later production
candidate would also need the default-GJ regression screen; earlier inactive
sort-code regressions remain a warning against relying only on local gains.
Evidence: unroll-one-cover-diagnostics/lazy-prober-feasibility.txt.


## Published root groups sharply reduce late cover candidates

The bounded group diagnostic is complete and independently accepted. Its patch
3e2ea4b2500a189e49acaebe68c7d8af757f00c984fab9c63a13cf3e881d37f0 adds
observations atop the exact readiness stack; binary egglog-cover-groups is
f15853c48ead7c2be8b58aedead0da3b8596dbf75dfae3f6803eb09bafafa93f. All
174 core tests pass in three configurations, formatting passes, and original
source/index/status bytes are restored. All six captures match their own full
report oracles: 16,494 ordered entries and 628 target scopes. Hard cross-mode
fingerprints agree; resize retains its known 16 cross-GJ count differences and
equal final sizes. These observations add no semantic-equivalence claim.

Actual sorted scalar keys confirm the motivating late selectivity. Resize
iteration 68 has one right-hand key and one physical exact-root candidate per
call; iteration 71 has six keys and six candidates per call. Intersecting those
groups with the current covers retains one and six physical candidates in total
across original cover sizes 2,369+33 and 2,375+33. Hard32/128 iteration 69 each
retain two candidates; Hard32 iteration 61 retains seven from eight raw rows
selected by seven keys. Hard128 iteration 61 has 19 keys, beyond the diagnostic
cap of eight, and is deliberately unobserved. Per-entry summaries aggregate two
calls and do not associate each intersection count with a particular cover size.
Candidate counts are not output counts: payload fanout can produce more matches.

Of 720 probe observations, 600 expose supported sorted views: 550 exceed the cap
and 50 qualify. Forty-four eligible exact roots are already published, six are
unready. The 44 observations visit 166 keys, 268 raw physical rows and 121 current
cover members across summed cover sizes 33,799. These totals repeat observations
and sometimes count a cover under both probes; they are not a whole-query work
estimate. The key cap does not bound row visits, and group reads can warm caches.
No elapsed measurements from this instrumented binary are used.

The unconstrained-root shortcut is unavailable: all 720 observations have a
cache but no existing (table, HeaderConstraintId0) root. Inherited readiness
metadata matches the predecessor exactly after excluding pointers and elapsed
fields. Thus a reuse-only execution experiment needs a modest way to retain the
eligible cover's original exact root. No prefilter policy is implemented yet;
root-retention, group lookup, membership, sorted-union and partition costs still
need a design review and timing. The observation cap is not a production gate.

Artifacts under /tmp/fused-lab: cover-prefilter-groups/{manifest.json,
runner-manifest.json,cover-groups.patch}; cover-group-diagnostics/{audit.json,
independent-final-audit.json,independent-final-audit.txt,interpretation.txt,
interpretation-data.json,independent-interpretation-crosscheck.json}. All 40
provenance pins, 11 patches, group accounting and interpretation independently
pass. No production code from this experiment is retained.

## Lazy fused-prober prototype frozen for measurement

The temporary lazy-prefix patch is frozen after three source/fixture reviews:
/tmp/fused-lab/lazy-fused-probes/lazy-probes.patch SHA
 aab8cef6a1ea0e63b7c2b272782528956f45bd60e6f611bd780cb43b7d6449e4.
Its exact ten-base stack is first-eight PK-v2 plus original probe-order switch
and one-live-cover, with no trace/readiness/rejected sort policy. LAZY_PROBES1
starts with an empty probe prefix and acquires another descriptor only after a
row passes every existing descriptor. Fully built rows retain the original
probe loop. ORDER_PROBES1 falls back to eager acquisition and sorting.

The independent scalar/tuple relational fixture now places failures before its
first success; a fourth header-restricted empty-output rule leaves the prefix
permanently incomplete. Other variants preserve slow/stale filtering, payload
multiplicity, later success/draining and one/four-worker checks. Previously
frozen fixtures remain untouched. No builds or measurements have run yet. A
same-binary eager/lazy local comparison and broad high-survival checks will
establish whether saved acquisition outweighs the added completion check and
possible deferred shared-index initialization.


## Lazy acquisition passes correctness and improves local rejection searches

The frozen lazy-prefix experiment builds successfully. All 176 core tests pass
in five configurations (off, lazy, ORDER1 eager fallback, selected one-cover and
selected live-Min), all 19 bridge tests and formatting pass, and original source,
index and status bytes are restored exactly. Binary egglog-lazy-fused-probes SHA
5cce2eb36abd5c4786bced8efe6949bd43c121d1a529eea39dbed163fbe2ef29 uses the
exact ten-base stack plus aab8cef6, without tracing or readiness instrumentation.

The balanced same-binary local comparison accepts all 70 processes: two models,
five treatments, five timed rounds plus two warmups. Every treatment occupies
each timed position exactly once, and every pair appears in both orders (2/3).
CPU24, j1, --no-decomp, 10-second/32-GiB bounds; eager/lazy treatments differ only
in LAZY_PROBES. All 273,000 ordered entries and 8,190 selected entries match their
own and cross-treatment full count/changed/aggregate/updated/can_stop/final-size
oracles. All 62 provenance pins, source restoration, commands and flags pass the
independent audit. No samples are discarded or rerun.

Direct eager-to-lazy target sums (milliseconds):

| Plan | Llama eager/lazy | Change | Qwen eager/lazy | Change |
| --- | --- | --- | --- | --- |
| One-cover | .587963 / .453463 | -22.876% | .568086 / .462321 | -18.618% |
| Selected live-Min | .445067 / .293439 | -34.069% | .393232 / .298119 | -24.188% |

Every sum and each individual orientation improves in all five pairs and both
order groups. One-cover state0 improves 31.38%/25.29% and state1 9.99%/9.27%;
live-Min state0 improves 45.15%/31.81% and state1 12.39%/11.91% (Llama/Qwen).
These remain zero-output rejection searches. The prior trace's unused-acquisition
lead now has measured local evidence, without identifying fresh index builds or
attributing all gains to a specific acquisition suboperation.

Whole-query/wall changes are smaller and mixed. One-cover: Llama -.394%/-.283%,
Qwen +.508%/+.304%; live-Min: Llama -.605%/-.624%, Qwen -.262%/+.507%.
No broad workload or production gain is claimed. Next reuse the existing native
PureSize/MinCover panels, including positive-output CYK/Hard/eggcc/model rules,
to check the extra completion branch and any downstream cache-cost shifts.

Artifacts: lazy-fused-probes-build/{manifest.json,test-*.log};
lazy-fused-timing-prepared/{manifest.json,README.txt};
lazy-fused-timing/independent-timing-audit.{json,txt}. The candidate remains
unapplied to main; no production change is retained yet.

## Retained-root prefilter prototype design accepted

The concrete design in cover-group-diagnostics/retained-root-proposal.txt is
approved for a separate temporary prototype. A small immutable eligible-cover
list in PreparedJoinIndexes retains the actual initial shared root after normal
root acquisition. It adds no per-access state, recursive argument or BindingInfo
clone cost. Initial scope is SinglePlan filtered scalar mappings; decomposed and
materialized cases fall back. The estimated change is 120-180 production lines.

Execution retains eager probes and original ordering, selects an existing sorted
scalar view, and falls back for top partitions. It peeks only an already-published
empty-slow-family index for the exact retained cover root. Actual key count must
be smaller than the current cover; summed raw group lengths must also be strictly
smaller before any row enumeration. This uses no workload name or copied cap8.
The materialized union intersects current-cover membership and sorts by physical
RowId, then uses unchanged projection, slow/live checks, all probes/refinements,
frame draining and restoration. Empty results flow through ordinary cleanup.

These necessary-condition restrictions can save scans but are not a net-cost
model. Key lookup, sparse membership, sorting and sidecar overhead must be timed;
default GJ also needs an overhead screen. Parallel top-partition fallback avoids
workers partitioning different candidate universes as indexes become published.
The prototype is independent of lazy acquisition and diagnostics, and requires
an output/order oracle before measurement. No prefilter policy is retained yet.


Local lazy provenance clarification: the frozen 62-pin bundle pins the model
input and its original source, but did not separately pin the shared transitive
include tests/header/luminal-header.egg. The exact output oracles and recorded worktree
restoration evidence still pass; they are not a substitute for a historical
include hash. Frozen inputs are preserved rather than amended retrospectively.
The next native bundle explicitly pins the shared include and checks the chosen
inputs' include closure before execution.


## Native lazy-acquisition screen: broad within-strategy gains, confirmation next

The seven-case/twelve-panel same-binary screen completes all 120 runs. The frozen
saved-data audit accepts all 423,660 ordered entries, every own-strategy reference
and eager/lazy count/changed/aggregate/updated/can_stop/final-size comparison.
No panels or samples are excluded. All 74 pins include the chosen transitive
header input; no trace, one-cover selection or probe sorting is enabled.
CPU24, j1, --no-decomp, three timed pairs plus two warmups, both execution orders.
The binary remains 5cce2eb3. Historical native reports serve only as correctness
references; all timing comparisons use the same new binary.

| Case / strategy | Query change | Wall change |
| --- | --- | --- |
| Llama PureSize | -9.510% | -6.731% |
| Llama live-Min | -1.727% | -.671% |
| Qwen PureSize | -11.960% | -9.308% |
| Qwen live-Min | -2.084% | -2.005% |
| Hard32 PureSize | -6.894% | -6.254% |
| eggcc PureSize | -3.187% | -1.694% |
| eggcc live-Min | -1.767% | -1.748% |
| CYK PureSize | +2.009% | +1.710% |
| CYK live-Min | -.249% | +.032% |
| Selected RMS PureSize | -1.699% | -1.077% |
| Selected Whisper beta PureSize | +.951% | +.871% |
| Selected Whisper beta live-Min | +.914% | +.745% |

The first seven panels improve query and wall in all three pairs and both order
groups. Positive-output rules contribute: the two model matmul rules have 13
matches each and improve 13-17% under PureSize; eggcc's 537-match extracted-vector
helper improves 13.21% under PureSize and 1.61% under live-Min. Match counts show
successful paths execute, but do not establish per-cover survival rates.
RMS target improves 3.369% and the two selected Whisper targets improve 2.788%/
2.533%, each in all three pairs. CYK Pure's positive mean is concentrated in one
52.922-ms lazy sample versus 50.081-ms eager; Whisper whole-query means likewise
include slower isolated samples. Those samples remain in all aggregates. A fresh
five-pair confirmation across the same twelve panels is being prepared, including
these mixed cases. No broad speedup relative to default GJ is claimed: native
PureSize/MinCover remain separate strategies, often much slower overall.

Artifacts: native-lazy-probes-prepared/{run.py,audit.py,plan.json};
native-lazy-probes/independent-native-audit.{json,txt}. Prepared plan SHA
4775dcdbd959207176f97eed5458b46ae0e4f3384a93ca91def09b755bdad504.
No source changes from the lazy or retained-root experiments are applied to main.


## Native lazy acquisition confirmed; clean production trial is warranted

The fresh five-pair confirmation accepts all 168 processes, 593,124 ordered
entries and twelve panels, with zero exclusions. All 126 pins, transitive includes,
full own-strategy and same-mode screen references, commands/flags and chronology
pass. Same 5cce2eb3 binary, five timed pairs plus two warmups, CPU24/j1/no-decomp,
both pair orders; prior runs are correctness references only.

| Case / strategy | Query change | Wall change |
| --- | --- | --- |
| Llama PureSize | -10.525% | -7.472% |
| Llama live-Min | -2.942% | -1.747% |
| Qwen PureSize | -11.470% | -8.826% |
| Qwen live-Min | -2.740% | -2.020% |
| Hard32 PureSize | -7.422% | -5.891% |
| eggcc PureSize | -3.119% | -1.819% |
| eggcc live-Min | -.637% | -.948% |
| CYK PureSize | +.322% | +.153% |
| CYK live-Min | -.411% | -.406% |
| Selected RMS PureSize | -1.799% | -.287% |
| Selected Whisper beta PureSize | -1.452% | -.630% |
| Selected Whisper beta live-Min | +.449% | +1.040% |

Both models' Pure/live-Min query gains, Hard32 Pure and eggcc Pure improve all
five pairs and both order groups. Eggcc live-Min query is mixed (3/5 faster, order
groups disagree); CYK Pure is slightly slower in 4/5 query pairs. Whisper remains
mixed in query time; its live-Min wall is slower in 4/5 pairs and both orders.
RMS target improves 3.228%, Whisper targets 3.077%/4.177%, all five pairs each.
No samples are dropped, and no claim of improvement across every workload is made.

The screen's per-entry decomposition also limits the success-path inference.
Model GEMM and eggcc helper gains include both positive- and zero-output entries;
Hard32's largest gain is almost entirely zero-output entries. Successful outputs
exercise completed prefixes but do not establish high per-cover survival rates.
See native-lazy-probes/screen-interpretation.{txt,json} for that decomposition.

A clean flag-free lazy acquisition draft is now justified for production testing,
with no diagnostic planner stack or strategy changes. It preserves the reviewed
prefix algorithm and adds an independent fixture with more than 256 successful
cover frames after initial failures to force intermediate draining. It is not
built or retained yet. The next builder will create a fresh control before applying
it, then run core/bridge/format checks and restore source bytes. Default GJ root11,
math iter12 and external ground-truth screens remain required before retention.
The existing bff505 control has equivalent source (only journal commits since its
build and identical saved user diff), but a fresh control gives stronger provenance.

Local non-target interpretation is saved at lazy-fused-timing/journal-note.txt and
non-target-interpretation.json. Repeatable losses in dtype_prop Add and later
direct-sigmoid-fusion occur in different ruleset calls from expr unroll, so the
run-scoped packed-trie cache does not explain them as a same-invocation transfer.
Persistent catalog initialization/refresh or other preparation remains unproven.
These later costs and the existing local shared-header provenance caveat remain
part of the evidence; the frozen local bundle is unchanged.

Confirmation artifacts: native-lazy-confirm-prepared/{run.py,audit.py,plan.json};
native-lazy-confirm/independent-native-confirm-audit.{json,txt}. Prepared plan SHA
abd533eda2aa2bf3ffb29ab7e52d916ebfaa0682c9628d9eaf610789807adeae.


## Clean lazy candidate builds with a fresh production control

The flag-free candidate is frozen as lazy-fused-production/lazy-production.patch
SHA 3be662d32c175d53e254540f1996f85e177e717b24e95741459647f34e3e6500.
It applies directly to production plus the preserved user diff, with no prototype
base stack. Runtime code starts with an empty original-order prober prefix and
acquires a later descriptor only when reached. The independent fixture now has
320 additional successful cover rows after initial failures, checks actual initial
physical order and initial DVO selection, and independently counts over 256
successful cover frames so intermediate draining and subsequent prefix reuse occur.

The fresh-control builder succeeds with 167 core tests, 19 bridge tests and fmt.
It records the same rustc1.91.0/Cargo toolchain, empty compiler override flags,
Cargo.lock, actual Cargo binary artifact paths, and control-before-patch command
sequence. Independent audit accepts all 18 pins and exact source/index/status
restoration; original user changes remain intact. No production patch is applied.

Fresh frozen binaries:
- Control egglog-lazy-production-control SHA
  97459cbb4efaae7cb1e8e467b8f125b3a1f150db65f716b41340de30572ebd5d.
- Candidate egglog-lazy-production SHA
  fbe4a0d79398ef3a72260bcd88954b6227e0ac014aaaa942c0a7fc0bd2b6ccc2.

Artifacts: lazy-production-build-prepared/manifest.json (SHA
3711ef46ecd01a12d2c9bd59e7312720f9807067603dd03c3e9acd1d422ce4cf),
lazy-production-build/{manifest.json,independent-build-audit.json,toolchain.json,
compiler-artifact-control.json,compiler-artifact-candidate.json,test-*.log}.
The report-backed default-GJ root11 plus math-iter12 paired screen is being prepared.
Its wall time will include report/trailer overhead and will not be mislabeled as
unmodified-input wall time. No clean-candidate performance result exists yet.

The independent retained-root prototype also freezes after source and fixture
reviews: retained-cover-prefilter/retained-cover-prefilter.patch SHA
aba4b94d51793d3c9892e0bae39b1f440a4b4b74bf7f43694543ca7e6d46681c,
manifest fda87e26505a241735c1a883a5aac47f60c0c7286123f4197744cb3ae7c2f217.
It remains separate from lazy acquisition, based on first-eight PK-v2 plus original
probe-order switch. The fixture forces actual published-root reuse on real Dense
and Sparse current subsets, independent row/action order, stale/slow filters and
fallbacks. Explicit original CoverRanges test partition bypass via serial calls
inside one/four-worker pools; they do not simulate concurrent publication races.
This prototype has not yet been built or measured and is not retained.

## Clean lazy production screen exposes default-GJ regressions

The frozen clean validation bundle (plan SHA
623e66193f0383ef58917518da8237b3bedab83fdee00dcc564b96484c3a108c)
accepts all 154 root11 processes and 981,652 ordered entries, with no exclusions.
Fresh control 97459cbb and candidate fbe4a0d7, default planner, five timed pairs
plus two warmups, both execution orders, CPU24/j1/no-decomp. All 93 pins,
transitive inputs, own historical references and current cross-binary counts,
changed flags, aggregate flags and final table sizes pass.

Summed mean query time increases .698%; summed report-backed wall increases
.335%. Hard128 query increases 1.372% and wall 1.022%, all five pairs each;
eggcc query increases 2.476%, all five; Hard32 query increases .935%, all five.
Other cases are mixed: Python query improves 1.472%, Llama .639%, paged Llama
.679%; most wall changes are small. The patch touches only the filtered fused
branch, and prior default-GJ censuses found that branch inactive. This makes
code-generation sensitivity plausible, but does not prove its cause. A fresh
15-pair Hard128/eggcc confirmation is prepared; external81 is held pending
resolution. No sample is dropped and no candidate is retained.

Math at exactly iteration 12 separately accepts all 14 processes and 4,928
ordered entries. Query improves .804% (4/5 pairs, both order groups), while wall
increases .625% (mixed 2/5 faster and order groups disagree). The measured wall
includes report and declared print-size/stats overhead; this is not an
unmodified-input, no-report wall result.

Artifacts: lazy-production-validation-prepared/{run.py,audit.py,plan.json};
lazy-production-validation/independent-{root11,math12}-audit.{json,txt}.

## Retained cover-root prefilter passes tests and a first timing screen

The separate retained-root builder passes 175 core tests in each of off, on,
and PK-v2-on modes, 19 bridge tests and fmt. The exact original user source diff,
index and status are restored. Frozen binary egglog-retained-cover-prefilter SHA
19ee7ebe163445483fae5f8cfaa5cdb809a96ddf0527b3a780424607885e1be2.
This is the first-eight PK-v2 plus original probe-order stack and the retained
root patch; it contains no lazy acquisition or timing trace.

The frozen three-pair screen (plan SHA
d10acaa764a5e3ba865ce6881d21103290f57429d75c74781c33d47b0bf8ff4a)
accepts 45 processes, 123,705 ordered entries and all 52 provenance pins. It
uses one binary, archived GJ-wait/PK-v2 settings, and changes only the prefilter
flag between PK off/on. All own-label and within-PK outputs are exact. Resize's
16 known cross-planner entry-count differences remain explicit; final table
sizes agree. Driver exit2 for resize reflects that cross-planner distinction,
not an own-plan failure.

| Case | PK on/off query | PK on/off wall | Ramp/Ramp target |
| --- | --- | --- | --- |
| Hard32 | -.422% (3/3 faster) | +.110% (0/3 faster) | -15.325% |
| Hard128 | +.231% (1/3 faster) | -.677% (2/3 faster) | -16.433% |
| resize | -2.001% (3/3 faster) | -.774% (3/3 faster) | -10.322% |

The late Ramp/Ramp entries improve 82.744%/59.723% in Hard32 and
96.168%/93.717% in resize. Hard128's iteration61 is unchanged (-.012%) while
iteration69 improves 70.511%. Equal-lanes targets improve 12.292%, 13.034%,
and 22.003%; divisible-lanes is mixed. Whole query/wall versus same-binary
GJ-wait remains roughly neutral; this is a local access-path lead, not a
production promotion. A separate diagnostic is being prepared to observe the
actual prefilter decisions and row conservation; timing alone does not prove
which calls activated. No counters will be added to this frozen timing binary.

Artifacts: retained-cover-build/{manifest.json,test-*.log};
retained-cover-timing-prepared/{run.py,audit.py,plan.json};
retained-cover-timing/independent-timing-audit.{json,txt}.

## Clean lazy candidate rejected after fresh default-GJ confirmation

The separately frozen confirmation bundle (plan SHA
44bbd6eb00e437a16b73e5e0803058d01bbfcba3d8cdddbb65abfd72e3d9fbe5)
accepts all 68 reports and 270,096 ordered entries. Same clean frozen binaries,
flags, inputs and resources, 15 fresh timed pairs plus two warmups, exact own
historical/current screen and current cross-binary outputs; all 60 pins pass.

Fresh Hard128 query increases 1.172% (median 1.235%, 14/15 slower) and report
wall .465% (13/15 slower). Eggcc query increases .980% (median .807%, 10/15
slower) and wall .598% (9/15 slower). Both order groups regress in each metric.
For explicitly labeled combined-20 context, including the initial five pairs,
Hard128 query is +1.222% (19/20 slower), wall +.603% (18/20); eggcc query is
+1.347% (15/20), wall +.583% (13/20). The fresh 15 are the confirmation;
pooling replaces neither experiment and removes no sample.

Fresh per-rule localization: Hard128 adds 3.556ms query, with ConvolutionShuffle+
adding 1.924ms (54.1%), almost entirely zero-output entries; iteration61/ordinal9
alone adds 1.688ms. Eggcc adds 1.462ms, with Project/Gamma interval-union adding
1.415ms (96.8%), split .812ms zero-output and .603ms positive-output. These locate
the measured cost without establishing its cause. The changed filtered-fused
arm is inactive in the inspected default GJ plans; no code-layout tuning is
attempted. The standalone production patch is rejected and remains unapplied.
Its within-native fused-path benefits remain an experimental lead for future
planning work. External81 stays unrun; its draft preparation is incomplete.
The report-backed wall numbers are not original-input/no-report wall claims.

Artifacts: lazy-production-regression-confirm-prepared/{run.py,audit.py,plan.json};
lazy-production-regression-confirm/{independent-root11-audit.json,
independent-root11-audit.txt,interpretation.json,interpretation.txt};
interpret-lazy-production-regression-confirm.py.

The retained-root timing also has an independent raw-data recheck: all 45
reports, 123,705 entries and 52 pins pass. Its three Ramp targets save .661ms
in Hard128 but other rules add 1.375ms; the largest convolution/WMMA offsets
are .455/.321/.255/.195ms. Resize's two main targets save 1.447ms, of which
only .162ms is in the two highlighted late entries. Therefore the separate
activation diagnostic must inspect complete target invocations, not infer the
whole gain from those late entries. Details: retained-cover-timing/
{independent-interpretation.json,journal-note.txt}.

A read-only planning assessment found no new justified general static gate
from live MinCover membership plus full-PK proof alone. A distinct possible
follow-up is a single precompiled alternate chosen when fresh per-focus
headers prove a cover singleton. This may avoid the earlier per-recursion
synthetic-stage cost, but candidate choice, registration overhead and scope
are still under review; no implementation or performance claim exists.

## Actual retained-root activation and scan conservation confirmed

The isolated activation patch fa225f6b050ba152fecf7aa682125913cc1422e4629cd2d47c9e3ddd561aef6d
adds fixed counters to the exact retained-root implementation, with no new index
lookup, key enumeration, scan or policy change. Source manifest ba9b7850 and
builder manifest dde81147 are frozen. The builder passes 175 core tests in three
configurations, 19 bridge tests and fmt, resolves the actual Cargo JSON executable,
and restores every worktree/index/status byte. Diagnostic binary
cb4bc6e21d2c7d367f9b5da1374009e7495331ce079f3e5133a0806ceac2f435.

Six PK-off/on captures use all three exact Ramp descriptions and every entry,
not only late entries. Runner manifest 2cade8770458c3963d7c3ef9ca0b4862bad7061633d36c48a8f0c118d06fd0da
pins 57 files. Both integrated and independent raw-data audits accept six reports,
16,494 full ordered entries and 628 target entries. Own untraced references and
current off/on complete fingerprints agree. Logical stages, physical positions,
source-size summaries and completed frames agree; requested/projected/visited/
failed rows never increase. All reason counts and aggregate conservation pass.

| Case | Filtered calls | Applied calls | Source/projected off | Selected/projected on | Completed frames, both |
| --- | --- | --- | --- | --- | --- |
| Hard32 | 118 | 47 | 19,301 | 5,693 | 2,472 |
| Hard128 | 118 | 44 | 49,969 | 16,762 | 5,750 |
| resize | 124 | 56 | 83,917 | 17,625 | 11,608 |

These are cumulative observations across filtered-stage calls, not distinct
table rows or whole-engine work. Off calls all report disabled. On fallback
reasons (no scalar / driver not smaller / unpublished / raw not smaller) are
8/47/8/8 for Hard32, 8/52/10/4 for Hard128, and 9/52/5/2 for resize. Applied
empty/nonempty counts are 8/39, 5/39 and 12/44. No missing-root, missing-family,
partition, identity-conflict or counter-overflow case occurs.

Hard128 iteration61's two Ramp/Ramp calls decline because the selected root
family is unpublished: 19 driver keys each, source sizes 62 and 1,409. Their
1,471 projected rows remain unchanged. Iteration69 applies and reduces 1,475
rows to two. Resize iterations68/71 reduce 2,402 to one and 2,408 to six, with
completed frames unchanged. Across full resize invocations, equal-lanes drops
44,360 to 8,205 projected rows (30/64 calls applied), and Ramp/Ramp drops
39,557 to 9,420 (26/60). The two late Ramp/Ramp entries account for only 4,803
of 66,292 removed rows, 7.25%; most observed activation is elsewhere.

This confirms the proposed access path operates and preserves successful work.
Performance remains the separate counter-free three-pair result recorded above;
no diagnostic duration prices the benefit or setup/cache costs. Artifacts:
retained-cover-activation/{activation.patch,manifest.json,README.txt};
retained-cover-activation-build/{manifest.json,independent-build-audit.json};
retained-cover-activation-prepared/{collect.py,audit.py,runner-manifest.json};
retained-cover-activation-diagnostics/{audit.json,independent-activation-analysis.json}.

The next singleton-cover metadata census passes root and independent source
review. It records the first proof-valid member of the initial live-MinCover
size ordering, then observes current physical table/individual header sizes
at normal cached focus refresh. It compiles no alternative, registers no extra
families, and changes no executed stages. Its log action ids are not unique
across skipped focuses/rulesets: opportunity counts will use chronological
per-description observations, with full-report correctness audited separately.
No census engine run has occurred yet.

## Singleton metadata census built; alternate-ready driver idea lacks evidence

The metadata-only singleton census freezes as singleton-cover-census.patch SHA
daf9e4d586787c078400e44a9811287d8b74ca5ba75ffccc159a7ed733096bf7,
source manifest 5615fc66f60b037d134972bb343e73197e9979f81695379b3a011d841e1fa02e.
Its exact base is first-eight PK-v2 plus original probe-order and one-cover
ablation patches. All their policy switches are off for the census. No lazy,
retained-root or activation-trace implementation is included.

Builder manifest 925783cc1357f92cd0b9bdf2180d9233b7e72a55088ec0d122a843247878ff37
pins 15 files and eleven patches. Both observer-off/on core configurations pass
175 tests, bridge passes 19, and fmt passes. The actual Cargo artifact freezes as
egglog-singleton-cover-census SHA
9df75277bfbd2d83e5513d3574b64183e270b86315a3abae923c4abc6234575d.
Original executor/fixture bytes at the base boundary and full source/index/status
restoration pass. No census engine capture has run yet. Root11 plus math12
collection is being prepared with full own report oracles and separately audited
chronological metadata; none of its times will be performance evidence.

A distinct small prefilter idea was checked against the saved readiness/group
records before implementation: prefer an already-published scalar driver's cover
family over the absolute smallest driver's unpublished family. Every one of the
23 actual unpublished declines aligns with archived calls where BOTH cover-family
observations were Unallocated (Hard32 eight, Hard128 ten, resize five). In Hard128
Ramp/Ramp iteration61/ordinal1, both calls have supported Packed scalar drivers
with 540 left keys and 19 right keys, and both corresponding cover families are
unallocated in both calls. Thus choosing the other ready driver is unsupported
for these cases: the archives show none. This is saved-data evidence, not an
extra lookup added to the measured or activation binaries. No policy trial is
started from aggregate readiness alone.

## Singleton opportunity census: first pass and bounded Python recovery

The unchanged census binary 9df75277 runs the default GJ program with only
metadata observation enabled. Frozen collector manifest 27ca3498 pins 91 files,
the current bench.py eleven inputs and their transitive includes, exact declared
print trailers, and math's sole run11-to-run12 edit. Root and two independent
reviews accept the collector and parser. All runs use no-decomp, one worker,
CPU24 and a 32GiB address-space bound; diagnostic times are not performance data.

Eleven first attempts pass full own ordered-count/changed/aggregate/updated/
can-stop/final-size oracles, covering 34,998 report entries. Python reaches the
30-second watchdog without a report: its preserved stderr contains 486,231,600
bytes and 644,557 lines, including a truncated final line. This remains a failed
attempt, with no correctness or timing conclusion. A separately frozen recovery
runner bdcbe87d pins 157 files and retries only Python for 120 seconds with the
same binary, flags and input. It writes raw output directly to disk and validates
metadata line by line; eight valid/eight malformed synthetic cases and complete
Hard32 parser-equivalence checks pass. The combined audit must recheck all twelve
full report oracles and identify the eleven reused captures plus one retry.

Partial accepted metadata reaches the intended local wins: Llama unroll-Add
trips4 state0/state1 each have 58 complete singleton focuses, and Qwen each has
59. All choose atom1 LoopStart with compile_rows1 and live columns [3,0,1,2,4,5].
These are metadata opportunities, not aligned executor entries or proven nonempty
roots. Whisper's corresponding candidate has compile_rows2 and no selected focus
among 99 complete focuses per state. Math has zero selected focuses among 304
complete observations. No beta/cublaslt description is selected in Llama, Qwen
or Whisper. Python remains pending, so this is not yet a twelve-case acceptance.

The next isolated prototype will compile one full-live-PK cover prefix with a GJ
suffix alongside the normal program, then choose it only from fresh singleton
table/header constituents before publishing an execution plan. A forced-GJ
control will compile/register both programs to expose that overhead. No recursive
stage conversion or executor dispatch change is planned. Separately, a five-panel
native MinCover/PureSize screen is being prepared for the retained-root prefilter;
its local PK gains alone do not establish gains under whole-native planning.

The Python recovery completes, and the combined audit accepts all twelve cases
and 70,470 full ordered entries. Independent rechecking verifies all 157 pins and
reparses all 971,632 metadata lines byte-exact to the compact records/dictionaries:
3,238 compilation records, 968,394 focus records and 12,677 complete singleton
opportunities. Of the latter, 6,591 have only a singleton header and 6,086 have
both singleton table and header. Another 51,038 skipped focuses have a singleton
constituent but correctly do not count as selected. The original Python timeout
is still preserved and excluded; no diagnostic time is used.

Final coverage confirms zero selected beta/cublaslt focuses across all 56 such
descriptions in each of the five current model inputs. This is an observation
on these inputs, not a structural future-data exclusion. PagedLlama also selects
58 complete Add4 focuses per state, and Qwen3MoE 67; Whisper selects none.
Python supplies 8,060 opportunities across 226 descriptions, including 5,869
header-only witnesses. Many selected candidates were empty at compilation.
CYK's six opportunities all use headers of initially empty P/B tables. The
prototype must handle growth from empty to singleton to many rows without
depending on declaration-time cardinality.

Alternative compilation is much broader than execution opportunity: each model
has 423 eligible compilation records out of 449, while only 31--39 descriptions
ever select. This makes the dual-registration forced-GJ control essential.
Artifacts: singleton-cover-census-recovered/{audit.json,independent-raw-audit.json,
interpretation.json,interpretation.txt}; raw first captures and timeout remain in
singleton-cover-census-diagnostics, and compact chronological records are under
singleton-cover-census-recovered/parsed. No alternate has yet been measured.

## Retained-root prefilter: native MinCover gain, PureSize regression confirmed

The five-panel screen freezes plan a625b426 (65 pins), using unchanged timing
binary 19ee7ebe. Hard32 PureSize, Llama PureSize/live-MinCover and eggcc
PureSize/live-MinCover each receive three timed pairs plus two warmups. Original
eager probe order and archived native flags remain fixed; PREFILTER0/1 is the
only within-panel difference. All 50 reports and 208,490 ordered entries pass
both own native references and current off/on fingerprints, independently checked.
No historical timing or native-versus-GJ timing comparison is made.

| Panel | Query change | Report-backed wall change | Faster query pairs |
| --- | ---: | ---: | ---: |
| Hard32 PureSize | -1.392% | -1.050% | 3/3 |
| Llama PureSize | +5.149% | +3.565% | 0/3 |
| Llama live-MinCover | -18.692% | -12.520% | 3/3 |
| eggcc PureSize | -0.502% | -0.232% | 2/3 |
| eggcc live-MinCover | -1.682% | -0.691% | 3/3 |

A fresh Llama-only confirmation freezes c4716a8b (70 pins), five timed pairs
plus two warmups per strategy, preserving the pilot separately. All 28 reports
and 107,156 ordered entries independently match historical own-mode and exact
same-binary pilot references. PureSize regresses query +4.691% and wall +3.613%
in every pair; live-MinCover improves query -18.132% and wall -12.510% in every
pair. Both execution-order groups agree in direction. This is a substantial
within-native gain with a repeatable adverse case, not grounds for promotion.

Pilot attribution locates PureSize's +26.483ms: +21.908ms (82.7%) is in zero-output
entries, with 255 rules slower and 205 slower in every pair. Two GEMM descriptions
add 8.651ms and batch-collapse merge A-contiguous B-broadcast adds 2.062ms.
Live-MinCover saves 87.888ms, including 48.301ms in nine grow-FE rules and 14.244ms
in direct-sigmoid-scaled-marker. The same grow-FE-B-lhs-Mul rule goes from
22.327 to 5.343ms under MinCover (-76.1%), but from 25.108 to 26.483ms under
PureSize. These timings identify a useful same-rule plan contrast; they do not
establish activation or the cause of failed-prefilter overhead.

A separate bounded diagnostic is being prepared for six exact Llama gain/loss
descriptions. It will reuse the existing activation counters, distinguish observed
singleton cover sources, and count only operations already performed by the
retention setup. No additional index lookup, policy or timing claim is introduced.
Artifacts: native-retained-prefilter{,-confirm}/{audit.json,independent-raw-audit.json};
native-retained-prefilter/interpretation.json. Original user source remains exact.

## Cached singleton prototype: first fixture failure isolated before measurement

The source package freezes as cached-singleton-cover.patch 5259ace2, manifest
de7a5d45, with about 140 added production lines and one cached-API fixture. Root
and independent source review accept the exact ten-patch base, full-PK proof,
separate planning state, both-family registration and fresh-header selection.
Builder manifest 093806eb pins 15 files/eleven patches and expects 176 core tests
in off/control/select modes plus 19 bridge tests. No timing run has occurred.

The first build fails in OFF mode: 175 tests pass, the new fixture expects zero
matches and observes one. No candidate binary is created. Saved worktree/index/
status restoration is byte-exact, independently verified. A focused rerun of the
unchanged frozen fixture with RUST_BACKTRACE=1 locates the failure at its final
constant-only witness deletion, query/tests.rs line682. All earlier empty/growth,
header-intersection and stale R/S cases pass before that assertion. The focused
rerun also restores the original source exactly. A minimal production-base
reproduction is being prepared; no cause or candidate correctness conclusion is
assumed yet. Frozen failed source and logs remain intact under
cached-singleton-cover-build and cached-singleton-cover-fixture-debug.

The minimal test-only reproduction 07fc28f0 now fails on the production base,
without any experimental patches. After deleting the only W(7) row, W has live
length zero, physical length one and an empty live scan, but the refreshed
EqConst header still contains one row. The cached all-constant witness publishes
and produces one match. This confirms an existing baseline liveness bug, rather
than a singleton-selection failure. The reproduction and its failing log remain
under constant-witness-stale-repro/production-run; source/index/status restore
exactly. This unrelated production path is not changed in this experiment.

A separate v2 package (patch 83a057b0, manifest d72e8c11) preserves the production
implementation byte-for-byte and changes only the final witness fixture to
clear_table, asserting both live and physical length zero before checking the
empty-header early return. The earlier stale R/S tests remain unchanged. Original
failed source/build artifacts remain immutable. V2 awaits its own build and
full off/control/select checks before any measurement.

The native activation diagnostic builds successfully as binary c3e31c15 from
source delta 98614b6a and builder manifest 3e37e60c. All three core configurations
pass 175 tests, bridge passes 19, and fmt passes. The builder freezes the actual
Cargo artifact and restores full source/index/status exactly. The observer adds
source zero/singleton counts and retention-loop work counters at existing
operations; four Llama PureSize/live-MinCover off/on captures are next. These
instrumented runs will establish activation, not performance.

## Native activation explains different opportunities; singleton v2 validates

Four frozen native diagnostic captures (manifest 18bbcf9c, 69 pins) pass all own
full report oracles: 15,308 ordered entries and 684 entries for the six selected
Llama descriptions. Independent raw replay also accepts all counter equations,
identity/overflow checks and within-strategy conservation. These runs are untimed
evidence; the earlier separate timing confirmation remains the performance source.

Across those six descriptions PureSize attempts the helper 585,858 times, with
242,237 singleton cover observations. It avoids only 8,092 of 3,013,364 requested
rows (0.269%); all 2,534 applications yield empty covers. Neither grow-FE lhs rule
nor batch-collapse applies the filter at all. Live-MinCover attempts it 140,308
times and avoids 2,295,078 of 8,179,542 requested rows (28.059%) over 5,248
applications. All successful frame totals remain unchanged within each strategy.
For grow-FE-B-lhs-Mul, MinCover's projected scans fall from 1,169,004 to 228,831;
PureSize's 538,189 scans stay unchanged. This establishes the difference in saved
work; it does not measure helper time or prove that singleton calls explain the
regression. Source and reason summaries cannot be paired per call without further
bounds. Artifacts: native-retained-activation-diagnostics/{audit.json,
independent-raw-audit.json,independent-acceptance-note.txt}.

The separate cached-singleton v2 build (manifest 471630bc) passes 176 core tests
in each of off/control/select, 19 bridge tests, and fmt. Binary 102a9e03 is frozen
from the actual Cargo artifact. Both root and independent review verify exact
source/index/status restoration. Production plan/query bytes match the original
prototype; only the documented final fixture case differs. The six with-plan
Llama/Qwen captures (manifest 9ab56fa1, 58 pins) now pass their collector's 23,400
full ordered-result checks; independent raw and plan interpretation follow before
the root11 plus math12 three-mode timing screen.

Independent six-plan replay accepts all 58 pins, all 23,400 ordered results and
all 234 Add4 plan comparisons. OFF/CONTROL reported programs are equal throughout.
SELECT changes 805 entries across 39 descriptions in Llama and 756 across 33 in
Qwen. Every observed changed prefix is filtered FusedIntersect, with LoopStart
accounting for 612 and 624 respectively. Add4 entries all return zero matches;
other changed entries include nine positive Llama entries (70 matches) and seven
positive Qwen entries (69 matches). These are logical plan observations, not
physical execution counts. Saved interpretation and independent raw audit are in
cached-singleton-cover-plans.

The native activation summaries further bound applied singleton calls to exactly
zero: every nonempty applied-cover size summary has minimum greater than one.
Thus a source-size-at-most-one prefilter bypass would skip 242,237 observed Pure
calls (41.35%) and 83,107 live-Min calls (59.23%) without losing any recorded
application. It would still incur root retention/setup and leave many other
fallbacks unchanged. Pure's leading decline counts are 265,911 driver-not-smaller,
261,103 unpublished and 55,067 no-scalar. This motivates one separate minimal
bypass experiment; it does not establish a measured gain or justify a threshold
sweep. The singleton alternate screen remains separate.

## Cached singleton timing: local model gains, broad policy rejected

Frozen screen 34865a2c pins 143 files and uses binary 102a9e03 throughout.
Root11 then math12 run serially, with three timed repetitions and two warmups
per off/control/select mode, CPU24/j1/no-decomp. All 180 reports and 1,057,050
ordered entries pass full own clean-GJ and current-mode output checks. Independent
raw replay accepts every pin, command, result fingerprint and paired statistic.
No sample or benchmark is excluded. The original source remains restored.

| Case | SELECT/OFF query | SELECT/OFF report wall | SELECT/CONTROL query |
| --- | ---: | ---: | ---: |

| hardboiled_conv1d_32 | +0.371% | +0.353% | -0.828% |

| hardboiled_conv1d_128 | +2.075% | +1.588% | +2.054% |

| luminal-llama | -4.189% | -0.215% | -3.014% |

| python_array_optimize | +2.669% | +0.026% | +2.580% |

| cykjson | +0.520% | +0.434% | -0.347% |

| eggcc-extraction | -3.209% | -1.374% | +0.888% |

| llama | -1.965% | +0.608% | -1.961% |

| paged_llama | -0.457% | +0.070% | -1.281% |

| qwen | -1.502% | +0.261% | -2.242% |

| qwen3_moe | -1.469% | +0.753% | -1.438% |

| whisper | +15.803% | +9.015% | +15.701% |

| math12 | -0.049% | -1.276% | -0.580% |


The root11 sum of case means regresses SELECT/OFF query +4.671% and wall +1.518%;
SELECT/CONTROL query +5.068% and wall +1.456%. Llama/Qwen query gains hold in all
three pairs, but net report wall does not improve. Eggcc's mean net gain largely
comes from CONTROL/OFF drift, not consistent selection benefit. Math's query is
essentially neutral, and its wall mean is order-sensitive (median -0.013%); with
zero census opportunities, this does not support a fused-plan speedup claim.

Whisper is decisively adverse: all three SELECT/CONTROL pairs lose, in both
orders. One zero-output rule, unroll LessThan body trips=4 state=1, grows from
0.7713ms to 51.3978ms (+50.6265ms), accounting for 97.76% of the whole-query
penalty. Its state0 counterpart improves 0.542ms. The fixed census candidate for
both is atom9 LessThan, compile physical size1, four PK columns and live columns
0..4, with 99/99 complete singleton witnesses. This is independent metadata by
description, not an ActionId-to-execution mapping. Late state1 ordinal2 entries
lose most: iteration140 grows about 10.35us to 5.603ms; iteration134 about 10.09us
to 5.286ms. Every aligned result remains zero in all three pairs. Singleton cover
size alone therefore does not bound the cost of the resulting execution program.
Separate Whisper reported-plan captures are being prepared to locate the cause.

This broad singleton policy is not a production candidate and does not proceed
to the external81 performance suite. Preserve its local gains and sharply defined
failure for investigation; no name-based exclusion or threshold tuning is applied.
Artifacts: cached-singleton-performance/{audit-root11.json,audit-math12.json,
independent-root11-raw-audit.json,independent-math12-raw-audit.json,
independent-root11-interpretation.txt,raw-rule-attribution.json}.

## Whisper singleton regression: identical reported shapes, physical cause pending

Two full-context CONTROL/SELECT with-plan captures freeze manifest bd06a254 with
71 pins, using the unchanged singleton binary 102a9e03 and current timing input,
flags and own references. Both 6,040-entry reports pass exact own and cross-output
checks, independently audited (12,080 entries total). SELECT changes 639 reported
programs; every observed changed prefix is filtered FusedIntersect.

Both LessThan4 states have 99 zero-output entries and one distinct reported plan
per mode. GJ has 23 stages; the alternate has 22. Its new logical first stage
binds LessThan physical columns0..4 and has exactly ONE scalar Op column0 probe.
It replaces original stages17 (the private LessThan keys scan) and18 (the scalar
output join); all other reported instructions remain byte-identical and in order.
The entire state0/state1 reported programs are equal after bijective renaming of
generated variables in both modes. This rules out a visible reported-shape or
multi-probe-width difference, not different atom identities, constraints, internal
maps, physical DVO order or actual work: those are absent from Plan JSON.

There is no evidence here for wasted acquisition of later probes because this
prefix has only one probe. A bounded physical trace is being prepared to separate
prefix/index/refinement work from later traversal before proposing an execution
change. A separate offline structural check will ask how much opportunity comes
from binding multiple shared variables versus moving one shared key and several
private RHS keys forward. It is not an implemented gate or a named-rule exclusion.
Artifacts: cached-singleton-whisper-plans/{audit.json,independent-raw-audit.json,
independent-target-programs.txt}.

The retained-cover source<=1 bypass separately freezes patch 2727e2e3, manifest
8ea45c64 and builder c55acb9f. It adds only an early optional decline before driver
selection, preserving ordinary fused scanning/probing. The expanded existing
fixture explicitly covers a ready empty scalar driver with a singleton cover:
OFF can produce Some(empty), while ON falls back and still returns zero matches.
Matched singleton, zero current, multi-row, stale/slow and partition cases remain.
All four core configurations pass 175 tests, bridge passes 19 and fmt passes. Actual
Cargo binary 441330a0 is frozen, with full original source/index/status restored and
independently accepted. Its 45-run three-mode timing screen is prepared separately
from the rejected singleton-plan policy.

## Prefilter singleton bypass: no convincing incremental gain

Frozen timing plan aaeeb0cf pins 66 files and runs binary 441330a0 in three
separate panels: Llama PureSize, Llama live-MinCover and Hard128 PKv2. Each has
three timed repetitions plus two warmups of prefilter-disabled, original-prefilter
and guarded-prefilter modes. All 45 reports and 155,205 ordered entries match
full archived own-mode and current cross-mode oracles. Independent raw replay
accepts every command, input/include, metric and paired/order statistic. No
samples or cases are excluded; no cross-binary times are compared.

| Panel | Guard/original query | Faster pairs | Guard/disabled query | Guard/disabled report wall |
| --- | ---: | ---: | ---: | ---: |
| Llama PureSize | +0.139% | 2/3 | +5.668% | +4.962% |
| Llama live-MinCover | -0.457% | 2/3 | -18.204% | -12.199% |
| Hard128 PKv2 | +0.280% | 1/3 | +0.004% | -0.138% |

Pure's incremental result changes direction by execution order, and its guarded
prefilter still loses to disabled in all three pairs. MinCover preserves its
substantial original prefilter gain but offers no compelling incremental guard
gain. The bypass therefore does not resolve the observed PureSize overhead and
is not promoted. Its observed reduction in futile singleton attempts did not
establish a useful net timing improvement. No further threshold or low-level
variant sweep is warranted from this result. Keep the bounded Whisper physical
trace as the next investigation. Artifacts: retained-cover-singleton-guard-timing/
{audit-screen.json,independent-screen-raw-audit.json}.

The source rules explain a distinction hidden by the reported table names: in
state0, LoopStart occupies the outer body ICons head and LoopInput's Op the inner
head; state1 swaps those positions. This is visible at tests/header/
luminal-header.egg lines5258--5289. It establishes different connectivity, not
which physical work causes the cost. State1 ordinal2 entries alone contribute
44.953ms of its 50.626ms penalty. The trace will retain atom identities and actual
stage positions rather than infer them from normalized Plan JSON.

An offline fixed-candidate restriction to at least two distinct shared variables
keeps 748/805 changed Llama entries (29 descriptions, observed selected-entry
query delta -0.813ms) and 713/756 Qwen entries (24 descriptions, -0.861ms).
It vetoes 57 Llama entries (-0.0002ms) and 43 Qwen entries (+0.0028ms). For Whisper
it keeps only 32/639 entries (seven MSub descriptions, +0.009ms) and vetoes 607
(25 descriptions, +50.204ms), including the good and bad LessThan4 orientations.
These are associations from the current run, not predicted rerun gains; changing
shared index use can change other work. The check vetoes the fixed candidate and
does not search for a replacement cover. Dominant Hard128 Float and Python
Program_function_two shapes have multiple shared variables, so this restriction
does not obviously address those losses. No gate is implemented or promoted.
Artifact: cached-singleton-performance/offline-shared-variable-gate.{json,txt}.


## Whisper singleton physical trace: late filtering multiplies traversal

The observational build adds only bounded counters to the exact singleton-v2
stack. All 176 core tests pass in off, trace-control and trace-select, all 19
bridge tests and formatting pass. Actual Cargo artifact 35482a0d is frozen;
source, index and status restore exactly. Both independent build reviews pass.
Frozen capture manifest a1b8943c pins 61 files. Two full-context, untimed captures
pass independent raw auditing of all 12,080 ordered report entries, all reported
plans, complete own/cross results and 396 exactly aligned target scopes. No
counter shape is truncated. Each target has 14 roots; two entries per mode/state
stop at an empty header. Diagnostic durations are not performance evidence.

State1 CONTROL rejects all 1,274 visits to the LessThan/Op scalar join at physical
position 11. SELECT rejects all 93,418 visits to the replacement filtered fused
stage at position 13. Every selected probe sees the same-sized already-packed
Op source: three rows and three keys. That stage performs zero central packed
builds and zero catalog refresh callbacks. Before it, unchanged list stages now
expand work: old9/new10 emits 201,188 frames, old8/new9 performs 201,188 payload
scans, and old11/new12 executes 99,502 times. There are additional downstream
builds, but repeated traversal already explains a concrete source of extra work;
this is not evidence for expensive construction of the singleton probe index.

State0 behaves differently: SELECT executes the fused check at physical position
1 on 80 visits, returns singleton Op subsets 68 times and rejects 12 times. Its
later joins then reject early. The actual repeated-atom relationships distinguish
the two orientations, despite alpha-equivalent reported programs. In particular,
the LoopStart-head join shares the initially constrained outer ICons atom only
in state1. The initial SELECT ordering puts the fused stage last in both states;
runtime sorting changes its eventual position. Per-stage histograms do not
reconstruct every sort decision or establish a full causal timing decomposition.

The source exposes a specific asymmetry: scalar Intersect uses the maximum
refinement credit over all inputs, while FusedIntersect uses only its cover's
credit. Thus the Op refinement that favors the original scalar check need not
favor the replacement fused check. Logical-prefix credit accounting and modulo-
three cadence also remain possible contributors. A narrow max-input-credit
ablation will isolate this mechanism without changing either of those policies.
This revives an old bundled idea, not a new one: EGGLOG_FUSED_REFINE_MAX appeared
in math12-prune-screen together with six other runtime flags plus dead-column
pruning and payload deferral; that result did not isolate its effect. The next
experiment changes only filtered-fused priority on the exact singleton-v2 base.
It does not yet promote the rejected singleton policy or claim a speedup.

Artifacts: whisper-singleton-trace-build/{manifest.json,
independent-build-audit.json,runtime-independent-build-audit.json};
whisper-singleton-trace-diagnostics/{audit.json,independent-raw-audit.json,
independent-stage-interpretation.json,independent-stage-interpretation.txt}.


## Isolated maximum-input refinement: repairs Whisper, no broad net gain

Patch ce9a9cb7 changes only the filtered-fused refinement score on the exact
cached-singleton-v2 stack. It takes the maximum credit across the cover and all
probes, leaving scalar/scan-only scores, logical-prefix seeding, cadence, index
acquisition and plan generation unchanged. The earlier multi-flag REFINE_MAX
experiment did not isolate this change. Two forced-mode ordering tests cover
initial/recursive probe credit and scalar/scan-only invariance. Build manifest
8d0783d5 passes 178 core tests in three configurations, 19 bridge tests and fmt.
Actual Cargo binary 9a893bcc and exact source/index/status restoration are
independently verified. No experimental source is retained in the worktree.

Frozen timing plan 9aa30673 pins 241 files. Three same-binary treatments are GJ
(singleton off, max off), unchanged singleton selection (max off), and singleton
selection with max on. Root11 uses three timed repetitions and two warmups per
mode, both relative execution orders, CPU24/j1/no-decomp. All 165 reports and
1,051,770 ordered entries pass independent full own-mode and cross-mode result
checks; every paired statistic is reproduced. No cases or samples are excluded.
Prior reports are correctness references only. Report wall includes trailers
and serialization. Three pairs are a screen, not broad confirmation.

| Case | Max/singleton query | Max/GJ query | Max/GJ report wall |
| --- | ---: | ---: | ---: |
| hardboiled_conv1d_32 | -0.041% | +0.008% | +0.163% |
| hardboiled_conv1d_128 | +0.845% | +1.644% | +1.292% |
| luminal-llama | +3.865% | +0.175% | +0.398% |
| python_array_optimize | -0.283% | +0.814% | -0.190% |
| cykjson | -0.693% | -0.050% | -0.626% |
| eggcc-extraction | +0.483% | +1.873% | +0.857% |
| llama | -0.060% | -1.446% | +1.579% |
| paged_llama | +0.216% | -0.357% | +0.506% |
| qwen | +0.408% | -1.134% | +1.224% |
| qwen3_moe | -0.301% | -1.334% | +1.427% |
| whisper | -13.543% | -0.492% | -0.250% |

Whisper max/singleton improves query 13.543% and wall 8.232%, all three pairs
in both orders. The targeted state1 rule falls from 51.804436ms to 0.915228ms
(-98.233%), recovering the traversal regression. It remains slower than GJ's
0.781033ms by 0.134195ms; state0 retains its 0.553660ms saving versus GJ.
The two-state total improves 20.689% versus GJ. This supports the lost-probe-
credit diagnosis without claiming that every downstream cost is measured.

Luminal loses its earlier singleton gain: max/singleton query +3.865%, with
row rms norm contributing +0.2521ms (+12.66%, all three pairs). Llama and Qwen
keep net query gains and substantial local Add4 gains, but their wall means
regress. Hard128 and eggcc max/GJ query and wall lose in all three pairs. The
sum of case means remains worse than GJ: query +0.397%, wall +0.430%. Relative
to the rejected singleton policy it improves query 4.168% and wall 1.185%,
largely by removing the known Whisper regression. The combined policy is not
promoted or sent to external81. No math repeat follows this rejected broad
screen: the earlier census found no singleton choices in math12.

A separate bounded scope test will disable singleton selection and compare
max off/on in five established native panels: Hard32 PureSize, Llama PureSize
and live-MinCover, eggcc PureSize and live-MinCover. This asks whether the same
ordering correction helps existing filtered programs without dual-plan overhead.
It uses exact own native oracles; no native/GJ equality or cross-binary timing
claim is assumed. This is not a production promotion or another threshold sweep.

Artifacts: fused-refinement-max-build/{manifest.json,independent-build-audit.json};
fused-refinement-max-performance/{audit-root11.json,independent-root11-raw-audit.json,
independent-target-interpretation.json,independent-target-interpretation.txt}.


## Native max-input refinement: opposing local wins, general policy rejected

Frozen plan 16ea2d59 pins 69 files and runs the already-tested 9a893bcc binary.
Singleton selection is disabled. Within each established native context, only
REFINE_MAX off/on changes; bridge/payload/live-cover settings retain their own
accepted references. Three timed repetitions plus two warmups, CPU24/j1/
no-decomp, five-second watchdog and 32GiB limit yield 50 successful reports.
Independent replay accepts all 208,490 ordered entries, five panels, full own
native and current off/on outputs, provenance and whole/per-rule paired stats.
No GJ-equivalence or cross-binary timing comparison is used. Nothing is excluded.

| Native panel | Max/off query | Max/off report wall | Faster query pairs |
| --- | ---: | ---: | ---: |
| hard32/pure | +21.006% | +16.998% | 0/3 |
| llama/pure | +17.021% | +12.121% | 0/3 |
| llama/min-live | +65.903% | +45.470% | 0/3 |
| eggcc/pure | +2.078% | +1.443% | 0/3 |
| eggcc/min-live | -0.030% | -0.060% | 1/3 |

All four losing panels regress in all three pairs and both execution orders.
Eggcc live-Min is neutral: median query +0.015%, only one faster pair. This
rejects global maximum-input refinement independently of singleton selection
or dual-program overhead. No confirmation run or external suite is warranted.

There are nevertheless large, consistent local wins. Hard32 Pure saves 29.616ms
(-89.50%, all three pairs) in a WMMA rule with 16 matches, but two convolution
rules add 89.610ms and 26.351ms. Llama Pure saves 51.405ms and 51.279ms
(about 78.8%, all three pairs) in two cublaslt GEMM rules, each with 13 matches.
Its zero-output batch-collapse and batch-matmul rules instead add 185.804ms
and 41.970ms. Live-Min loses 114.600ms on the expert-index marker, 66.143ms
on the scaled-sigmoid marker and 55.785ms on FE/FE Mul. Eggcc Pure adds
4.252ms to the vector extraction helper. Max credit can therefore expose useful
access orders while badly promoting other paths; a positive local result does
not establish a usable global policy or identify a safe admission condition.

The separate root11 attribution also cautions against blaming every regression
on that rule's own chosen fused stage: Hard128's dominant rule has only one
selection among 151 archived census focuses, not mapped to the hot entry;
eggcc's dominant Gamma rule has zero selections among seven. Unused alternates
retain compiled stages/family vectors and repeat registration, but do not build
indexes or allocate per-run prepared state unless selected. Newly interned
families can enlarge shared cache-node metadata for other rules; no saved
family-count telemetry quantifies that effect. A structural compile-scope
question remains: avoid an alternate whose cover has no relational probe and
only duplicates an existing private/RHS scan. This is under offline assessment,
not an implemented gate or a claimed explanation of timing.

Artifacts: native-refinement-max/{audit.json,independent-raw-audit.json,
independent-acceptance-note.txt}; fused-refinement-max-performance/
independent-loss-attribution.{json,txt}.
