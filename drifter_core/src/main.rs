use drifter_core::corpus::{Corpus, InMemoryCorpus};
use drifter_core::executor::{Executor, InProcessExecutor};
use drifter_core::feedback::{Feedback, UniqueInputFeedback};
use drifter_core::input::Input;
use drifter_core::mutator::{FlipSingleByteMutator, Mutator};
use drifter_core::observer::{NoOpObserver, Observer};
use drifter_core::oracle::{CrashOracle, Oracle};
use drifter_core::scheduler::{RandomScheduler, Scheduler};

use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use std::collections::HashMap;
use std::time::Instant;

fn my_harness(data: &[u8]) {
    if data.len() > 2 && data[0] == b'B' && data[1] == b'A' && data[2] == b'D' {
        panic!("BAD input detected by harness!");
    }
    if data.len() > 3 && data[0] == b'C' && data[1] == b'R' && data[2] == b'A' && data[3] == b'S' {
        panic!("CRASH input detected by harness!");
    }
}

fn main() -> Result<(), anyhow::Error> {
    let mut rng = ChaCha8Rng::from_seed([0u8; 32]);

    let mut mutator = FlipSingleByteMutator;
    let mut executor = InProcessExecutor::new(my_harness);
    let oracle = CrashOracle;
    let mut corpus: InMemoryCorpus<Vec<u8>> = InMemoryCorpus::new();
    let mut scheduler = RandomScheduler::new();
    let mut feedback = UniqueInputFeedback::new();

    let mut noop_observer = NoOpObserver;
    let mut observers_list: Vec<&mut dyn Observer> = vec![&mut noop_observer];

    let initial_seed_data = vec![b'G', b'O', b'O', b'D'];
    let seed_hash = md5::compute(initial_seed_data.as_bytes());
    feedback.known_hashes.insert(seed_hash.0);
    corpus.add(initial_seed_data, Box::new("Initial Seed".to_string()))?;

    feedback.init(&corpus)?;

    println!("Starting fuzz loop with basic Corpus, Scheduler, and Feedback...");
    let start_time = Instant::now();
    let mut executions = 0;
    let mut solutions_found = 0;

    let max_iterations = 50000;

    for i in 0..max_iterations {
        let (base_input_id, base_input_to_mutate) = match scheduler.next(&corpus, &mut rng) {
            Ok(id) => {
                let (input_ref, _meta_ref) = corpus.get(id).expect("Scheduler returned invalid ID");
                (id, input_ref.clone())
            }
            Err(_e) => {
                let generated = mutator.mutate(None, &mut rng, Some(&corpus))?;
                corpus.add(generated.clone(), Box::new("Generated Seed".to_string()))?;
                feedback
                    .known_hashes
                    .insert(md5::compute(generated.as_bytes()).0);
                (corpus.len() - 1, generated)
            }
        };

        let mutated_input = mutator.mutate(Some(&base_input_to_mutate), &mut rng, Some(&corpus))?;
        executions += 1;

        for obs in observers_list.iter_mut() {
            obs.reset()?;
        }
        let status = executor.execute_sync(&mutated_input, &mut observers_list);

        let mut observers_data_map = HashMap::new();
        for obs in observers_list.iter() {
            observers_data_map.insert(obs.name(), obs.serialize_data());
        }

        let mut is_solution = false;
        if feedback.is_interesting(&mutated_input, &observers_data_map, &corpus)? {
            feedback.report_interesting(mutated_input.clone(), &observers_data_map, &mut corpus)?;
        }

        if let Some(bug_report) = oracle.examine(&mutated_input, &status, None) {
            println!("\n!!! BUG FOUND (Execution {}) !!!", executions);
            println!("  Input: {:?}", bug_report.input);
            println!("  Description: {}", bug_report.description);
            println!("  Hash: {}", bug_report.hash);
            corpus.add(
                bug_report.input.clone(),
                Box::new(format!("Crash: {}", bug_report.description)),
            )?;
            is_solution = true;
            solutions_found += 1;
        }

        <RandomScheduler as Scheduler<Vec<u8>, InMemoryCorpus<Vec<u8>>>>::report_feedback(
            &mut scheduler,
            base_input_id,
            &"some_feedback_value",
            is_solution,
        );

        if i % (max_iterations / 100) == 0 && i > 0 {
            let elapsed = start_time.elapsed().as_secs_f32();
            let exec_per_sec = executions as f32 / elapsed;
            print!(
                "\rIter: {}/{}, Corpus: {}, Solutions: {}, Execs/sec: {:.2}",
                i,
                max_iterations,
                corpus.len(),
                solutions_found,
                exec_per_sec
            );
            use std::io::Write;
            std::io::stdout().flush().unwrap();
        }
    }
    let elapsed_total = start_time.elapsed();
    println!("\nFuzz loop finished in {:.2?}.", elapsed_total);
    println!(
        "Total Executions: {}, Corpus Size: {}, Solutions Found: {}",
        executions,
        corpus.len(),
        solutions_found
    );
    Ok(())
}
