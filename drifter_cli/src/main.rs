use drifter_core::config::{
    CorpusType, DrifterConfig, ExecutorType as ConfigExecutorType, default_iterations,
};
use drifter_core::corpus::{Corpus, InMemoryCorpus};
use drifter_core::executor::{
    CommandExecutor, CommandExecutorConfig, Executor, InProcessExecutor,
    InputDelivery as CoreInputDelivery,
};
use drifter_core::feedback::{Feedback, UniqueInputFeedback};
use drifter_core::mutator::{FlipSingleByteMutator, Mutator};
use drifter_core::observer::{NoOpObserver, Observer};
use drifter_core::oracle::{CrashOracle, Oracle};
use drifter_core::scheduler::{RandomScheduler, Scheduler};

use clap::Parser;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    #[clap(short, long, value_parser)]
    config_file: Option<PathBuf>,
    #[clap(long)]
    target_command: Option<String>,
    #[clap(short, long)]
    iterations: Option<u64>,
}

fn my_harness(data: &[u8]) {
    if data.len() > 2 && data[0] == b'B' && data[1] == b'A' && data[2] == b'D' {
        panic!("BAD input detected by harness!");
    }
    if data.len() > 3 && data[0] == b'C' && data[1] == b'R' && data[2] == b'A' && data[3] == b'S' {
        panic!("CRASH input detected by harness!");
    }
}

fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    let mut config = match cli.config_file {
        Some(config_path) => {
            println!("Loading configuration from specified path: {config_path:?}",);
            DrifterConfig::load_from_file(&config_path)?
        }
        None => {
            // No config file specified via CLI, load default
            let default_config_path = PathBuf::from("config.toml");
            if default_config_path.exists() {
                println!(
                    "No config file specified via CLI, loading default: {default_config_path:?}",
                );
                DrifterConfig::load_from_file(&default_config_path)?
            } else {
                println!(
                    "No config file specified and default 'config.toml' not found, using built-in defaults."
                );
                DrifterConfig::default()
            }
        }
    };

    if let Some(iterations) = cli.iterations {
        config
            .fuzzer
            .get_or_insert_with(Default::default)
            .max_iterations = iterations;
    }
    if let Some(target_cmd_str) = cli.target_command {
        if config.executor.executor_type == ConfigExecutorType::Command {
            let cmd_settings = config.executor.command_settings.get_or_insert_with(|| {
                drifter_core::config::CommandExecutorSettings {
                    command: Vec::new(),
                    input_delivery: drifter_core::config::ConfigInputDelivery::StdIn,
                    timeout_ms: 2000,
                    working_dir: None,
                }
            });
            if !cmd_settings.command.is_empty() {
                cmd_settings.command[0] = target_cmd_str;
            } else {
                cmd_settings.command.push(target_cmd_str);
            }
        } else {
            println!(
                "Warning: --target-command specified but executor type is not 'Command'. Override ignored."
            );
        }
    }

    println!("Effective configuration: {config:#?}");

    let mut rng = ChaCha8Rng::from_seed([0u8; 32]);

    let mut mutator: Box<dyn Mutator<Vec<u8>>> = Box::new(FlipSingleByteMutator);

    let mut executor: Box<dyn Executor<Vec<u8>>> = match config.executor.executor_type {
        ConfigExecutorType::InProcess => Box::new(InProcessExecutor::new(my_harness)),
        ConfigExecutorType::Command => {
            let cmd_settings = config.executor.command_settings.ok_or_else(|| {
                anyhow::anyhow!("Command settings missing for CommandExecutor type in config")
            })?;

            let core_input_delivery = match cmd_settings.input_delivery {
                drifter_core::config::ConfigInputDelivery::StdIn => CoreInputDelivery::StdIn,
                drifter_core::config::ConfigInputDelivery::File { template } => {
                    CoreInputDelivery::File(template)
                }
            };

            let exec_config = CommandExecutorConfig {
                command: cmd_settings.command,
                input_delivery: core_input_delivery,
                timeout: std::time::Duration::from_millis(cmd_settings.timeout_ms),
                working_dir: cmd_settings.working_dir,
            };
            Box::new(CommandExecutor::new(exec_config))
        }
    };

    let oracle: Box<dyn Oracle<Vec<u8>>> = Box::new(CrashOracle);

    let mut corpus: Box<dyn Corpus<Vec<u8>>> = match config.corpus.as_ref().unwrap().corpus_type {
        CorpusType::OnDisk => {
            let path = config
                .corpus
                .as_ref()
                .unwrap()
                .on_disk_path
                .clone()
                .ok_or_else(|| anyhow::anyhow!("on_disk_path missing for OnDiskCorpus"))?;
            println!(
                "Using OnDiskCorpus (Not yet implemented, falling back to InMemory). Path: {path:?}",
            );
            Box::new(InMemoryCorpus::<Vec<u8>>::new())
        }
        _ => Box::new(InMemoryCorpus::<Vec<u8>>::new()),
    };

    if let Some(corpus_conf) = &config.corpus {
        if let Some(seed_paths) = &corpus_conf.initial_seed_paths {
            for path in seed_paths {
                if path.is_file() {
                    let data = std::fs::read(path)?;
                    let meta: Box<dyn Any + Send + Sync> = Box::new(format!("Seed: {path:?}"));
                    corpus.add(data, meta)?;
                } else if path.is_dir() {
                    for entry in std::fs::read_dir(path)? {
                        let entry = entry?;
                        let file_path = entry.path();
                        if file_path.is_file() {
                            let data = std::fs::read(&file_path)?;
                            let meta: Box<dyn Any + Send + Sync> =
                                Box::new(format!("Seed: {file_path:?}"));
                            corpus.add(data, meta)?;
                        }
                    }
                }
            }
        }
    }
    if corpus.is_empty() {
        let default_seed_data = vec![b'I', b'N', b'I', b'T'];
        let meta: Box<dyn Any + Send + Sync> = Box::new("Default Initial Seed".to_string());
        corpus.add(default_seed_data, meta)?;
    }

    let mut scheduler: Box<dyn Scheduler<Vec<u8>>> = Box::new(RandomScheduler::new());
    let mut feedback_processor: Box<dyn Feedback<Vec<u8>>> = Box::new(UniqueInputFeedback::new());

    let mut noop_observer = NoOpObserver;
    let mut observers_list: Vec<&mut dyn Observer> = vec![&mut noop_observer];

    for i in 0..corpus.len() {
        if let Some((input_ref, _)) = corpus.get(i) {
            if let Some(uif) = feedback_processor
                .as_mut()
                .as_any_mut()
                .downcast_mut::<UniqueInputFeedback>()
            {
                uif.add_known_input_hash(input_ref);
            } else {
                // Generic feedback might have an init that scans the corpus.
            }
        }
    }
    feedback_processor.init(corpus.as_ref())?;

    let max_iterations = config
        .fuzzer
        .as_ref()
        .map_or(default_iterations(), |f| f.max_iterations);

    println!(
        "Starting fuzz loop for {} iterations with {} initial corpus items...",
        max_iterations,
        corpus.len()
    );
    let start_time = Instant::now();
    let mut executions = 0;
    let mut solutions_found = 0;

    for i in 0..max_iterations {
        let (base_input_id, base_input_to_mutate) =
            match scheduler.as_mut().next(corpus.as_ref(), &mut rng) {
                Ok(id) => {
                    let (input_ref, _meta_ref) =
                        corpus.get(id).expect("Scheduler returned invalid ID");
                    (id, input_ref.clone())
                }
                Err(_e) => {
                    if corpus.is_empty() {
                        break;
                    }
                    let generated = mutator.mutate(None, &mut rng, Some(corpus.as_ref()))?;
                    let meta: Box<dyn Any + Send + Sync> =
                        Box::new("Emergency Generated Seed".to_string());
                    let new_id = corpus.add(generated.clone(), meta)?;
                    if let Some(uif) = feedback_processor
                        .as_mut()
                        .as_any_mut()
                        .downcast_mut::<UniqueInputFeedback>()
                    {
                        uif.add_known_input_hash(&generated);
                    }
                    (new_id, generated)
                }
            };

        let mutated_input =
            mutator.mutate(Some(&base_input_to_mutate), &mut rng, Some(corpus.as_ref()))?;
        executions += 1;

        for obs in observers_list.iter_mut() {
            obs.reset()?;
        }
        let status = executor
            .as_mut()
            .execute_sync(&mutated_input, &mut observers_list);

        let mut observers_data_map = HashMap::new();
        for obs in observers_list.iter() {
            observers_data_map.insert(obs.name(), obs.serialize_data());
        }

        let mut is_solution = false;
        let _was_added_by_feedback = feedback_processor.as_mut().process_execution(
            &mutated_input,
            mutated_input.clone(),
            &observers_data_map,
            corpus.as_mut(),
        )?;

        if let Some(bug_report) = oracle.as_ref().examine(&mutated_input, &status, None) {
            println!("\n!!! BUG FOUND (Execution {executions}) !!!");
            println!("  Input: {:?}", bug_report.input);
            println!("  Description: {}", bug_report.description);
            println!("  Hash: {}", bug_report.hash);
            let bug_meta: Box<dyn Any + Send + Sync> =
                Box::new(format!("Crash: {}", bug_report.description));
            corpus.add(bug_report.input.clone(), bug_meta)?;
            if let Some(uif) = feedback_processor
                .as_mut()
                .as_any_mut()
                .downcast_mut::<UniqueInputFeedback>()
            {
                uif.add_known_input_hash(&bug_report.input);
            }
            is_solution = true;
            solutions_found += 1;
        }

        scheduler
            .as_mut()
            .report_feedback(base_input_id, &"some_feedback_value", is_solution);

        if i > 0 && i % (max_iterations / 100).max(1) == 0 {
            let elapsed = start_time.elapsed().as_secs_f32();
            let exec_per_sec = if elapsed > 0.0 {
                executions as f32 / elapsed
            } else {
                0.0
            };
            print!(
                "\rIter: {}/{}, Corpus: {}, Solutions: {}, Execs/sec: {:.2}   ",
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
    println!("\nFuzz loop finished in {elapsed_total:.2?}.");
    println!(
        "Total Executions: {}, Corpus Size: {}, Solutions Found: {}",
        executions,
        corpus.len(),
        solutions_found
    );

    Ok(())
}
