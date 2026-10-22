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
baseline to rank them. A separate 60-second diagnostic is running; PureSize
has also exceeded that bound. Existing free join is applied only to default GJ
rules by the temporary strategy switch; explicit rebuild strategies are intact.
Math has 4 one-atom rules (already MinCover), 18 binary rules, and 2 triangles.
Eight binary constructor-output joins are the main targets; their shared key
usually is not a primary key, so the reversal prototype does not help them.
Investigate repeated payload projection and grouped terminal execution while
preserving GJ's factorization, rather than applying whole-cover fusion blindly.
