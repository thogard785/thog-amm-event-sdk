//! Local CPU benchmarks; no RPC, sleeps, network, or fixture decoding in quote timings.
#[path = "../../fixtures/support.rs"]
mod support;

use event_driven_sdk::{CommitState, EventSynchronizer};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    time::{Duration, Instant},
};
use thogamm_model::{events, ExecutionContext, U256};

struct CountingAllocator;
static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
// Delegates allocation and deallocation, with the original layout, to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
        }
        System.realloc(ptr, layout, size)
    }
}

fn bench(name: &str, mut f: impl FnMut()) {
    let mut iterations = 1usize;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            f();
        }
        if start.elapsed() >= Duration::from_millis(20) {
            break;
        }
        iterations *= 2;
    }
    let mut samples = [0.0f64; 7];
    for sample in &mut samples {
        let start = Instant::now();
        for _ in 0..iterations {
            f();
        }
        *sample = start.elapsed().as_nanos() as f64 / iterations as f64;
    }
    ALLOCATIONS.store(0, Relaxed);
    COUNTING.store(true, Relaxed);
    for _ in 0..iterations {
        f();
    }
    COUNTING.store(false, Relaxed);
    let allocs = ALLOCATIONS.load(Relaxed) as f64 / iterations as f64;
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:42} median {:10.1} ns  min {:10.1} ns  allocs/op {allocs:.2}",
        samples[3], samples[0]
    );
}

fn main() {
    // cargo test --all-targets builds this harness without running measurements.
    if !std::env::args().any(|arg| arg == "--bench") {
        return;
    }
    let f = support::fixture();
    for world in [0, 1, 3] {
        let set = &f.samples[world];
        let model = support::model(f.proxy, f.baseFee, &set.snapshot);
        let tokens = model.state().tokens.len();
        println!(
            "world {world}: {tokens} tokens, {} categories, {} words, {} ABI bytes",
            model.state().categories.len(),
            model.state().words.len(),
            set.snapshot.len()
        );
        for kind in [0, 4] {
            let cases: Vec<_> = set
                .quotes
                .iter()
                .filter(|s| s.kind == kind && s.success)
                .collect();
            assert!(!cases.is_empty());
            println!(
                "  surface {kind}: cycling {} successful Solidity cases",
                cases.len()
            );
            let mut index = 0;
            bench(
                &format!(
                    "{tokens} tokens / exact {}",
                    if kind == 0 { "input" } else { "output" }
                ),
                || {
                    let s = cases[index % cases.len()];
                    index += 1;
                    let a = model.state().tokens[s.inputIndex as usize].token;
                    let b = model.state().tokens[s.outputIndex as usize].token;
                    let context = ExecutionContext {
                        gas_price: s.gasPrice,
                        fast_lane_hot: s.fastLaneHot,
                    };
                    let result = if kind == 0 {
                        model.quote_exact_input(black_box(a), black_box(b), black_box(s.amount))
                    } else {
                        model.quote_exact_output(
                            black_box(a),
                            black_box(b),
                            black_box(s.amount),
                            black_box(&context),
                        )
                    };
                    black_box(result.unwrap());
                },
            );
            let prepared: Vec<_> = cases
                .iter()
                .map(|s| {
                    model
                        .prepare(
                            model.state().tokens[s.inputIndex as usize].token,
                            model.state().tokens[s.outputIndex as usize].token,
                        )
                        .unwrap()
                })
                .collect();
            let mut index = 0;
            bench(
                &format!(
                    "{tokens} tokens / prepared exact {}",
                    if kind == 0 { "input" } else { "output" }
                ),
                || {
                    let s = cases[index % cases.len()];
                    let pair = &prepared[index % prepared.len()];
                    index += 1;
                    let context = ExecutionContext {
                        gas_price: s.gasPrice,
                        fast_lane_hot: s.fastLaneHot,
                    };
                    let result = if kind == 0 {
                        black_box(pair).quote_exact_input(black_box(s.amount))
                    } else {
                        black_box(pair).quote_exact_output(black_box(s.amount), black_box(&context))
                    };
                    black_box(result.unwrap());
                },
            );
        }
        let s = set
            .quotes
            .iter()
            .find(|s| {
                s.kind == 0
                    && s.success
                    && (1..=16).all(|d| {
                        model
                            .quote_exact_input(
                                model.state().tokens[s.inputIndex as usize].token,
                                model.state().tokens[s.outputIndex as usize].token,
                                s.amount / U256::from(d),
                            )
                            .is_ok()
                    })
            })
            .unwrap();
        let a = model.state().tokens[s.inputIndex as usize].token;
        let b = model.state().tokens[s.outputIndex as usize].token;
        println!(
            "  ladder pair {} -> {}, amount {} / 1..16",
            s.inputIndex, s.outputIndex, s.amount
        );
        bench(&format!("{tokens} tokens / 16 input amounts"), || {
            for divisor in 1..=16 {
                black_box(
                    model
                        .quote_exact_input(
                            black_box(a),
                            black_box(b),
                            black_box(s.amount / U256::from(divisor)),
                        )
                        .unwrap(),
                );
            }
        });
        bench(&format!("{tokens} tokens / prepare + 16 amounts"), || {
            let pair = model.prepare(black_box(a), black_box(b)).unwrap();
            for divisor in 1..=16 {
                black_box(
                    pair.quote_exact_input(black_box(s.amount / U256::from(divisor)))
                        .unwrap(),
                );
            }
        });
        bench(
            &format!("{tokens} tokens / decode + build snapshot"),
            || {
                black_box(support::polled_model(f.proxy, black_box(&set.snapshot)));
            },
        );
        bench(&format!("{tokens} tokens / clone model"), || {
            black_box(model.clone());
        });
        bench(&format!("{tokens} tokens / project next block"), || {
            black_box(
                model
                    .at_block(model.state().block.number + 1, f.baseFee)
                    .unwrap(),
            );
        });
    }
    let frames: Vec<_> = f
        .events
        .iter()
        .map(|event| {
            (
                support::model(f.proxy, f.baseFee, &event.beforeState),
                support::model_header(&support::model(f.proxy, f.baseFee, &event.afterState)),
                support::logs(event, f.baseFee),
            )
        })
        .collect();
    let mut index = 0;
    bench("apply one event block (mixed 11 stages)", || {
        let (before, header, logs) = &frames[index % frames.len()];
        index += 1;
        black_box(events::apply_block(before.state(), header.clone(), black_box(logs)).unwrap());
    });
    bench("synchronizer seed + 11 finalized blocks", || {
        let mut sync = EventSynchronizer::from_model(frames[0].0.clone()).unwrap();
        for (_, header, logs) in &frames {
            sync.on_head(header.clone(), CommitState::Proposed).unwrap();
            for log in logs {
                sync.on_log(log.clone()).unwrap();
            }
            black_box(
                sync.on_head(header.clone(), CommitState::Finalized)
                    .unwrap()
                    .unwrap(),
            );
        }
        black_box(sync);
    });
}
