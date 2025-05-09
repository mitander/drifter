use drifter_core::corpus::{Corpus, InMemoryCorpus};
use drifter_core::executor::{Executor, InProcessExecutor};
use drifter_core::feedback::{Feedback, UniqueInputFeedback};
use drifter_core::mutator::{FlipSingleByteMutator, Mutator};
use drifter_core::observer::{NoOpObserver, Observer};
use drifter_core::oracle::{CrashOracle, Oracle};
use drifter_core::scheduler::{RandomScheduler, Scheduler};

use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use std::any::Any;
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

    let initial_seed_data_1 = vec![b'G', b'O', b'O', b'D'];
    let seed_meta_1: Box<dyn Any + Send + Sync> = Box::new("Initial Seed 1".to_string());
    corpus.add(initial_seed_data_1.clone(), seed_meta_1)?;
    feedback.add_known_input_hash(&initial_seed_data_1);

    let initial_seed_data_2 = vec![b'S', b'A', b'F', b'E'];
    let seed_meta_2: Box<dyn Any + Send + Sync> = Box::new("Initial Seed 2".to_string());
    corpus.add(initial_seed_data_2.clone(), seed_meta_2)?;
    feedback.add_known_input_hash(&initial_seed_data_2);

    feedback.init(&corpus)?;

    println!("Starting fuzz loop with encapsulated Feedback...");
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
                println!(
                    "Warning: Scheduler failed, possibly empty corpus after initial seeds processed? This shouldn't happen with current logic."
                );
                let emergency_seed = vec![0u8];
                feedback.add_known_input_hash(&emergency_seed);
                corpus.add(
                    emergency_seed.clone(),
                    Box::new("Emergency Seed".to_string()),
                )?;
                (corpus.len() - 1, emergency_seed)
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
        let _was_added_by_feedback = feedback.process_execution(
            &mutated_input,
            mutated_input.clone(),
            &observers_data_map,
            &mut corpus,
        )?;

        if let Some(bug_report) = oracle.examine(&mutated_input, &status, None) {
            println!("\n!!! BUG FOUND (Execution {}) !!!", executions);
            println!("  Input: {:?}", bug_report.input);
            println!("  Description: {}", bug_report.description);
            println!("  Hash: {}", bug_report.hash);
            let bug_meta: Box<dyn Any + Send + Sync> =
                Box::new(format!("Crash: {}", bug_report.description));
            corpus.add(bug_report.input.clone(), bug_meta)?;
            feedback.add_known_input_hash(&bug_report.input);
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
            let exec_per_sec = if elapsed > 0.0 {
                executions as f32 / elapsed
            } else {
                0.0
            };
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
