use std::sync::{Arc, Mutex};

use smallvec::smallvec;

use super::super::join_tail::{BindingSet, MatchCounter};
use super::{ActionBuffer, InPlaceActionBuffer, expand_binding_sets};
use crate::{
    action::{ExecutionState, Instr, QueryEntry},
    common::Value,
    free_join::{Database, Variable, make_external_func},
    numeric_id::{DenseIdMap, NumericId},
    offsets::RowId,
    pool::Pooled,
    query::{ActionInfo, RuleSet},
    row_buffer::TaggedRowBuffer,
    table_shortcuts::v,
};

#[test]
fn factorized_batch_matches_scalar_expansion_at_batch_boundaries_and_stop() {
    for used_vars_empty in [false, true] {
        for prefill in [0, 127] {
            for factor_count in [0, 1, 2, 3] {
                for (inner_len, shared_rhs_var) in
                    [(0, false), (1, false), (129, false), (130, true)]
                {
                    for stop_at in [None, Some(0), Some(130)] {
                        let run = |fast| {
                            let [x, y, scalar, dead, dst] =
                                [2, 17, 33, 64, 65].map(Variable::from_usize);
                            let log = Arc::new(Mutex::new(Vec::<Vec<Value>>::new()));
                            let action_log = Arc::clone(&log);
                            let mut db = Database::new();
                            let logger = db.add_external_function(Box::new(make_external_func(
                                move |state, args| {
                                    let mut log = action_log.lock().unwrap();
                                    log.push(args.to_vec());
                                    if stop_at == Some(log.len()) {
                                        state.trigger_early_stop();
                                    }
                                    // Exercise rejected RHS rows as well as accepted matches.
                                    (args.first().copied() != Some(v(101))).then_some(v(1))
                                },
                            )));
                            let mut rules = RuleSet::default();
                            let mut instrs: Pooled<Vec<Instr>> = Default::default();
                            instrs.push(Instr::External {
                                func: logger,
                                args: if used_vars_empty {
                                    vec![QueryEntry::Const(v(42))]
                                } else {
                                    vec![x.into(), y.into(), scalar.into()]
                                },
                                dst,
                            });
                            let action = rules.actions.push(ActionInfo {
                                used_vars: if used_vars_empty {
                                    smallvec![]
                                } else {
                                    smallvec![x, y, scalar]
                                },
                                instrs: Arc::new(instrs),
                            });
                            let counter = MatchCounter::new(1);
                            let mut buffer = InPlaceActionBuffer {
                                rule_set: &rules,
                                match_counter: &counter,
                                batches: Default::default(),
                            };
                            let mut state =
                                ExecutionState::new(db.read_only_view(), Default::default());
                            let mut bindings = DenseIdMap::new();
                            for (var, value) in
                                [(x, v(10)), (y, v(20)), (scalar, v(30)), (dead, v(40))]
                            {
                                bindings.insert(var, value);
                            }
                            for _ in 0..prefill {
                                buffer.push_bindings(action, &bindings, || {
                                    state.seed().to_execution_state()
                                });
                            }
                            if stop_at == Some(0) {
                                state.trigger_early_stop();
                            }
                            let mut outer = TaggedRowBuffer::new_inline(2);
                            for i in 0..3 {
                                outer.add_row(RowId::from_usize(i), &[v(100 + i), v(900 + i)]);
                            }
                            let outer = Arc::new(outer);
                            let inner_width = if shared_rhs_var { 4 } else { 3 };
                            let mut inner = TaggedRowBuffer::new_inline(inner_width);
                            for i in 0..inner_len {
                                let values = [v(200 + i), v(800 + i), v(300 + i), v(400 + i)];
                                inner.add_row(RowId::from_usize(i), &values[..inner_width]);
                            }
                            let mut factors = BindingSet::new();
                            if factor_count == 3 {
                                factors.push((smallvec![scalar, dead], Arc::clone(&outer)));
                            }
                            if factor_count >= 2 {
                                factors.push((smallvec![x, dead], outer));
                            }
                            // Repeated y must use its final projected column, just like scalar binding.
                            if factor_count >= 1 {
                                let mut vars = smallvec![y, dead, y];
                                if shared_rhs_var {
                                    // The RHS reads x from both factors; the inner value wins.
                                    vars.push(x);
                                }
                                factors.push((vars, Arc::new(inner)));
                            }
                            for _ in 0..2 {
                                let handled = fast
                                    && buffer.try_push_factorized_batch(
                                        action,
                                        &mut bindings,
                                        &factors,
                                        &state.seed(),
                                    );
                                if fast {
                                    assert_eq!(
                                        handled,
                                        !used_vars_empty && (1..=2).contains(&factor_count)
                                    );
                                }
                                if !handled {
                                    expand_binding_sets(
                                        &mut buffer,
                                        action,
                                        &mut bindings,
                                        &factors,
                                        0,
                                        &state.seed(),
                                    );
                                }
                            }
                            let before_flush =
                                buffer.batches.get(action).map(|b| (b.n_runs, b.len));
                            buffer.flush(&mut state);
                            let output = log.lock().unwrap().clone();
                            if shared_rhs_var
                                && !used_vars_empty
                                && prefill == 0
                                && factor_count == 2
                                && stop_at.is_none()
                            {
                                // Two expansions of three outer rows, each with the full inner
                                // group, must never expose the outer x values 100, 101, or 102.
                                let expected = (0..6)
                                    .flat_map(|_| {
                                        (0..inner_len).map(|i| vec![v(400 + i), v(300 + i), v(30)])
                                    })
                                    .collect::<Vec<_>>();
                                assert_eq!(output, expected, "inner factor must overwrite RHS x");
                            }
                            (output, before_flush, counter.read_matches(action))
                        };
                        assert_eq!(
                            run(false),
                            run(true),
                            "empty={used_vars_empty} prefill={prefill} factors={factor_count} inner={inner_len} stop={stop_at:?}"
                        );
                    }
                }
            }
        }
    }
}
