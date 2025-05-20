use clap::Parser;
use drifter_core::config::{
    CommandExecutorSettings, CorpusType, DrifterConfig, ExecutorType, InProcessExecutorSettings,
    default_iterations,
};
use drifter_core::corpus::{
    Corpus, CorpusError, InMemoryCorpus, OnDiskCorpus, OnDiskCorpusEntryMetadata,
};
use drifter_core::executor::{
    CommandExecutor, CommandExecutorConfig, Executor, InProcessExecutor, InputDelivery,
};
use drifter_core::feedback::{Feedback, MockBitmapCoverageFeedback, UniqueInputFeedback};
use drifter_core::input::Input;
use drifter_core::mutator::{FlipSingleByteMutator, Mutator};
use drifter_core::observer::{MockCoverageObserver, Observer};
use drifter_core::oracle::{CrashOracle, Oracle};
use drifter_core::scheduler::{RandomScheduler, Scheduler, SchedulerError};
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use rohcstar::fuzz_harnesses::rohc_decompressor_harness;
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

fn dummy_harness(data: &[u8]) {
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
        Some(config_path) => DrifterConfig::load_from_file(&config_path)?,
        None => {
            let default_config_path = PathBuf::from("drifter_config.toml");
            if default_config_path.exists() {
                println!(
                    "No config file specified via CLI, loading default: {:?}",
                    default_config_path
                );
                DrifterConfig::load_from_file(&default_config_path)?
            } else {
                println!(
                    "No config file specified and default 'drifter_config.toml' not found, using built-in defaults."
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
        if config.executor.executor_type == ExecutorType::Command {
            let cmd_settings = config
                .executor
                .command_settings
                .get_or_insert_with(CommandExecutorSettings::default);
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
    if config.executor.in_process_settings.is_none() {
        config.executor.in_process_settings = Some(InProcessExecutorSettings::default());
    }

    println!("Effective configuration: {:#?}", config);

    let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
    let mut mutator: Box<dyn Mutator<Vec<u8>>> = Box::new(FlipSingleByteMutator);
    let oracle: Box<dyn Oracle<Vec<u8>>> = Box::new(CrashOracle);

    let mut executor: Box<dyn Executor<Vec<u8>>> = match config.executor.executor_type {
        ExecutorType::InProcess => {
            let settings = config
                .executor
                .in_process_settings
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "InProcessExecutor settings missing in config (should be defaulted)"
                    )
                })?;

            let harness_fn: fn(&[u8]) = match settings.harness_key.as_str() {
                "rohcstar_decompressor" => {
                    println!("Selected Rohcstar decompressor harness.");
                    rohc_decompressor_harness
                }
                "" | "default_dummy" => {
                    println!("Using default/dummy in-process harness.");
                    dummy_harness
                }
                unknown_key => {
                    return Err(anyhow::anyhow!(
                        "Unknown in-process harness key specified: '{}'",
                        unknown_key
                    ));
                }
            };
            Box::new(InProcessExecutor::new(harness_fn))
        }
        ExecutorType::Command => {
            let cmd_settings = config.executor.command_settings.ok_or_else(|| {
                anyhow::anyhow!("Command settings missing for CommandExecutor type in config")
            })?;
            let core_input_delivery = match cmd_settings.input_delivery {
                drifter_core::config::ConfigInputDelivery::StdIn => InputDelivery::StdIn,
                drifter_core::config::ConfigInputDelivery::File { template } => {
                    InputDelivery::File(template)
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

    let mut corpus: Box<dyn Corpus<Vec<u8>>>;
    let corpus_conf = config.corpus.unwrap_or_else(|| {
        println!("No [corpus] section in config, defaulting to InMemoryCorpus with default path.");
        drifter_core::config::CorpusConfig {
            corpus_type: CorpusType::default(),
            initial_seed_paths: None,
            on_disk_path: drifter_core::config::default_on_disk_path(),
        }
    });

    corpus = match &corpus_conf.corpus_type {
        CorpusType::OnDisk => {
            println!("Using OnDiskCorpus at path: {:?}", corpus_conf.on_disk_path);
            Box::new(OnDiskCorpus::new(corpus_conf.on_disk_path.clone())?)
        }
        CorpusType::InMemory => {
            println!("Using InMemoryCorpus.");
            Box::new(InMemoryCorpus::new())
        }
    };

    if let Some(seed_paths) = &corpus_conf.initial_seed_paths {
        if !seed_paths.is_empty() {
            match corpus.load_initial_seeds(seed_paths) {
                Ok(num_loaded) => println!("Loaded {} initial seeds from paths.", num_loaded),
                Err(e) => println!("Warning: Failed to load some initial seeds: {}", e),
            }
        } else {
            println!("initial_seed_paths was specified but empty in config.");
        }
    } else {
        println!("No initial_seed_paths specified in config.");
    }

    if corpus.is_empty() {
        println!(
            "Corpus is empty after attempting to load seeds, adding a default seed: [I,N,I,T]."
        );
        let default_seed_data = vec![b'I', b'N', b'I', b'T'];
        let meta: Box<dyn Any + Send + Sync> = Box::new(OnDiskCorpusEntryMetadata {
            source_description: "Default Initial Seed".to_string(),
        });
        corpus.add(default_seed_data, meta)?;
    }

    let mut scheduler: Box<dyn Scheduler<Vec<u8>>> = Box::new(RandomScheduler::new());

    let mut main_feedback: Box<dyn Feedback<Vec<u8>>> = Box::new(MockBitmapCoverageFeedback::new(
        "MockCoverageObserver".to_string(),
    ));
    // let mut main_feedback: Box<dyn Feedback<Vec<u8>>> = Box::new(UniqueInputFeedback::new());

    let mut coverage_observer = MockCoverageObserver::new();
    // let mut noop_observer = NoOpObserver::default();

    let mut observers_list_for_executor: Vec<&mut dyn Observer> = vec![
        &mut coverage_observer,
        // &mut noop_observer,
    ];

    for i in 0..corpus.len() {
        if let Some((input_ref, _)) = corpus.get(i) {
            if let Some(mbf) = main_feedback
                .as_any_mut()
                .downcast_mut::<MockBitmapCoverageFeedback>()
            {
                let hash_array = md5::compute(input_ref.as_bytes()).0;
                mbf.global_coverage_hashes.insert(hash_array);
            } else if let Some(uif) = main_feedback
                .as_any_mut()
                .downcast_mut::<UniqueInputFeedback>()
            {
                uif.add_known_input_hash(input_ref);
            }
        }
    }
    main_feedback.init(corpus.as_ref())?;

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
        let (base_input_id, base_input_to_mutate) = match scheduler.next(corpus.as_mut(), &mut rng)
        {
            Ok(id) => match corpus.get(id) {
                Some((input_ref, _meta_ref)) => (id, input_ref.clone()),
                None => {
                    eprintln!(
                        "Scheduler returned ID {} but corpus.get failed. Rescheduling.",
                        id
                    );
                    continue;
                }
            },
            Err(SchedulerError::CorpusEmpty) => {
                if corpus.is_empty() {
                    println!("Corpus empty, cannot schedule. Attempting to generate a new seed.");
                    if let Ok(generated) = mutator.mutate(None, &mut rng, Some(corpus.as_ref())) {
                        let meta: Box<dyn Any + Send + Sync> =
                            Box::new(OnDiskCorpusEntryMetadata {
                                source_description: "Emergency Generated Seed From Empty"
                                    .to_string(),
                            });
                        match corpus.add(generated.clone(), meta) {
                            Ok(new_id) => {
                                if let Some(mbf) = main_feedback
                                    .as_any_mut()
                                    .downcast_mut::<MockBitmapCoverageFeedback>()
                                {
                                    let hash_array = md5::compute(generated.as_bytes()).0;
                                    mbf.global_coverage_hashes.insert(hash_array);
                                } else if let Some(uif) = main_feedback
                                    .as_any_mut()
                                    .downcast_mut::<UniqueInputFeedback>()
                                {
                                    uif.add_known_input_hash(&generated);
                                }
                                (new_id, generated)
                            }
                            Err(e) => {
                                eprintln!("Failed to add emergency seed: {:?}. Stopping.", e);
                                break;
                            }
                        }
                    } else {
                        eprintln!("Mutator failed to generate seed from scratch. Stopping.");
                        break;
                    }
                } else {
                    eprintln!(
                        "Scheduler reported corpus empty, but corpus.is_empty() is false. Rescheduling."
                    );
                    continue;
                }
            }
            Err(e) => {
                eprintln!("Scheduler error: {:?}. Stopping.", e);
                break;
            }
        };

        let mutated_input =
            match mutator.mutate(Some(&base_input_to_mutate), &mut rng, Some(corpus.as_ref())) {
                Ok(inp) => inp,
                Err(e) => {
                    eprintln!(
                        "Mutation failed for input_id {}: {:?}. Skipping.",
                        base_input_id, e
                    );
                    continue;
                }
            };
        executions += 1;

        for obs in observers_list_for_executor.iter_mut() {
            obs.reset()?;
        }

        let status = executor.execute_sync(&mutated_input, &mut observers_list_for_executor);
        let mut observers_data_map = HashMap::new();
        for obs in observers_list_for_executor.iter() {
            observers_data_map.insert(obs.name(), obs.serialize_data());
        }

        let mut input_was_interesting_for_corpus = false;
        if main_feedback.is_interesting(&mutated_input, &observers_data_map, corpus.as_ref())?
            && main_feedback.add_to_corpus(mutated_input.clone(), corpus.as_mut())?
        {
            input_was_interesting_for_corpus = true;
            if corpus.len() % 10 == 0 || input_was_interesting_for_corpus {
                println!(
                    "\nNew coverage found! Input added. Corpus size: {}",
                    corpus.len()
                );
            }
        }

        let mut is_solution = false;
        if let Some(bug_report) = oracle.examine(&mutated_input, &status, None) {
            println!("\n!!! BUG FOUND (Execution {}) !!!", executions);
            println!("  Input (bytes): {:?}", bug_report.input.as_bytes());
            println!("  Description: {}", bug_report.description);
            println!("  Hash: {}", bug_report.hash);
            solutions_found += 1;
            is_solution = true;

            let crash_meta: Box<dyn Any + Send + Sync> = Box::new(OnDiskCorpusEntryMetadata {
                source_description: format!(
                    "Crash: {} (Exec: {})",
                    bug_report.description, executions
                ),
            });
            if !input_was_interesting_for_corpus {
                match corpus.add(bug_report.input.clone(), crash_meta) {
                    Ok(_) => println!("Added crashing input to corpus."),
                    Err(CorpusError::IoError(e)) if e.contains("already exists") => {}
                    Err(e) => eprintln!("Error adding crashing input to corpus: {:?}", e),
                }
            }
        }

        scheduler.report_feedback(
            base_input_id,
            &(input_was_interesting_for_corpus || is_solution),
            is_solution,
        );

        if i > 0
            && i % (max_iterations / 100).max(1) == 0
            && solutions_found == 0
            && !input_was_interesting_for_corpus
        {
            let elapsed = start_time.elapsed().as_secs_f32();
            let exec_per_sec = if elapsed > 0.0 {
                executions as f32 / elapsed
            } else {
                0.0
            };
            print!(
                "\rIter: {}/{}, Corpus: {}, Solutions: {}, Execs/sec: {:.2}   ",
                i + 1,
                max_iterations,
                corpus.len(),
                solutions_found,
                exec_per_sec
            );
            use std::io::Write;
            std::io::stdout().flush().unwrap();
        } else if solutions_found > 0 || input_was_interesting_for_corpus {
            let elapsed = start_time.elapsed().as_secs_f32();
            let exec_per_sec = if elapsed > 0.0 {
                executions as f32 / elapsed
            } else {
                0.0
            };
            println!(
                "\rIter: {}/{}, Corpus: {}, Solutions: {}, Execs/sec: {:.2}   ",
                i + 1,
                max_iterations,
                corpus.len(),
                solutions_found,
                exec_per_sec
            );
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
