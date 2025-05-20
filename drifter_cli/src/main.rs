use clap::Parser;
use drifter_core::config::{DrifterConfig, ExecutorType};
use drifter_core::corpus::{
    Corpus, CorpusError, InMemoryCorpus, OnDiskCorpus, OnDiskCorpusEntryMetadata,
};
use drifter_core::executor::{
    CommandExecutor, CommandExecutorConfig, Executor, InProcessExecutor, InputDeliveryMode,
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
use std::io::Write as StdIoWrite;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
    /// Path to the TOML configuration file for Drifter.
    #[clap(short, long, value_parser, verbatim_doc_comment)]
    config_file: Option<PathBuf>,

    /// Overrides the target command specified in the config file (first element of `command_with_args`).
    /// Only applicable if executor_type is "Command".
    /// Example: --target-command "./my_app" (arguments are preserved from config or need separate CLI flags)
    #[clap(long, verbatim_doc_comment)]
    target_command_override: Option<String>,

    /// Overrides the maximum number of fuzzing iterations from the config file.
    #[clap(short, long, value_parser, verbatim_doc_comment)]
    iterations_override: Option<u64>,
}

/// A simple dummy harness function for in-process fuzzing demonstrations.
/// Panics if the input contains "BAD" or "CRASH".
fn dummy_harness(data: &[u8]) {
    if data.len() > 2 && data[0] == b'B' && data[1] == b'A' && data[2] == b'D' {
        panic!("BAD input detected by dummy_harness!");
    }
    if data.len() > 3 && data[0] == b'C' && data[1] == b'R' && data[2] == b'A' && data[3] == b'S' {
        panic!("CRASH input detected by dummy_harness!");
    }
}

fn main() -> Result<(), anyhow::Error> {
    let cli_args = CliArgs::parse();

    let mut drifter_config = match cli_args.config_file {
        Some(config_path) => {
            println!("INFO: Loading configuration from: {:?}", config_path);
            DrifterConfig::load_from_file(&config_path)?
        }
        None => {
            let default_config_path = PathBuf::from("drifter_config.toml");
            if default_config_path.exists() {
                println!(
                    "INFO: No config file specified via CLI, attempting to load default: {:?}",
                    default_config_path
                );
                DrifterConfig::load_from_file(&default_config_path)?
            } else {
                println!(
                    "INFO: No config file specified and 'drifter_config.toml' not found. Using built-in defaults."
                );
                DrifterConfig::default()
            }
        }
    };

    if let Some(cli_iterations) = cli_args.iterations_override {
        drifter_config
            .fuzzer
            .get_or_insert_with(Default::default)
            .max_iterations = cli_iterations;
    }

    if let Some(cli_target_cmd_str) = cli_args.target_command_override {
        if drifter_config.executor.executor_type == ExecutorType::Command {
            let cmd_settings = drifter_config
                .executor
                .command_settings
                .get_or_insert_with(Default::default);

            if !cmd_settings.command.is_empty() {
                cmd_settings.command[0] = cli_target_cmd_str;
            } else {
                cmd_settings.command.push(cli_target_cmd_str);
            }
        } else {
            println!(
                "WARN: --target-command-override specified, but executor type is not 'Command'. Override ignored."
            );
        }
    }

    if drifter_config.executor.executor_type == ExecutorType::InProcess
        && drifter_config.executor.in_process_settings.is_none()
    {
        drifter_config.executor.in_process_settings = Some(Default::default());
    }

    println!(
        "INFO: Effective Drifter Configuration: {:#?}",
        drifter_config
    );

    let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
    let mut mutator: Box<dyn Mutator<Vec<u8>, ChaCha8Rng>> = Box::new(FlipSingleByteMutator);
    let oracle: Box<dyn Oracle<Vec<u8>>> = Box::new(CrashOracle);

    let mut executor: Box<dyn Executor<Vec<u8>>> = match drifter_config.executor.executor_type {
        ExecutorType::InProcess => {
            let settings = drifter_config
                .executor
                .in_process_settings
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("InProcess executor type selected, but in_process_settings are missing in config. This should have been defaulted earlier."))?;

            let harness_fn_to_run: fn(&[u8]) = match settings.harness_key.as_str() {
                "rohcstar_decompressor" => {
                    println!("INFO: Using Rohcstar decompressor in-process harness.");
                    rohc_decompressor_harness
                }
                "" | "default_dummy" => {
                    println!("INFO: Using default_dummy in-process harness.");
                    dummy_harness
                }
                unknown_key => {
                    return Err(anyhow::anyhow!(
                        "Unknown in-process harness_key specified in config: '{}'. Available keys: 'rohcstar_decompressor', 'default_dummy' (or empty).",
                        unknown_key
                    ));
                }
            };
            Box::new(InProcessExecutor::new(harness_fn_to_run))
        }
        ExecutorType::Command => {
            let cmd_settings_from_config = drifter_config.executor.command_settings.ok_or_else(|| {
                anyhow::anyhow!("Command executor type selected, but [executor.command_settings] table is missing or empty in config.")
            })?;

            let exec_input_delivery_mode = match cmd_settings_from_config.input_delivery {
                drifter_core::config::ConfigInputDelivery::StdIn => InputDeliveryMode::StdIn,
                drifter_core::config::ConfigInputDelivery::File { template } => {
                    InputDeliveryMode::File(template)
                }
            };

            let runtime_exec_config = CommandExecutorConfig {
                command_with_args: cmd_settings_from_config.command,
                input_delivery_mode: exec_input_delivery_mode,
                execution_timeout: Duration::from_millis(cmd_settings_from_config.timeout_ms),
                working_directory: cmd_settings_from_config.working_dir,
            };
            println!(
                "INFO: Using CommandExecutor with command: {:?}",
                runtime_exec_config.command_with_args
            );
            Box::new(CommandExecutor::new(runtime_exec_config))
        }
    };

    let corpus_config_from_drifter = drifter_config.corpus.unwrap_or_else(|| {
        println!("WARN: No [corpus] section in config, defaulting to InMemoryCorpus.");
        drifter_core::config::CorpusConfig::default()
    });

    let mut corpus: Box<dyn Corpus<Vec<u8>>> = match &corpus_config_from_drifter.corpus_type {
        drifter_core::config::CorpusType::OnDisk => {
            println!(
                "INFO: Using OnDiskCorpus at path: {:?}",
                corpus_config_from_drifter.on_disk_path
            );
            Box::new(OnDiskCorpus::new(
                corpus_config_from_drifter.on_disk_path.clone(),
            )?)
        }
        drifter_core::config::CorpusType::InMemory => {
            println!("INFO: Using InMemoryCorpus.");
            Box::new(InMemoryCorpus::<Vec<u8>>::new())
        }
    };

    if let Some(seed_paths_vec) = &corpus_config_from_drifter.initial_seed_paths {
        if !seed_paths_vec.is_empty() {
            match corpus.load_initial_seeds(seed_paths_vec) {
                Ok(num_loaded) => println!(
                    "INFO: Loaded {} initial seeds from specified paths.",
                    num_loaded
                ),
                Err(e) => eprintln!("WARN: Failed to load some initial seeds: {}", e),
            }
        } else {
            println!(
                "INFO: 'initial_seed_paths' was specified but empty in config. No seeds loaded from paths."
            );
        }
    } else {
        println!("INFO: No 'initial_seed_paths' specified in config.");
    }

    if corpus.is_empty() {
        println!(
            "INFO: Corpus is empty after attempting seed loading. Adding a default seed: vec![73, 78, 73, 84] (b\"INIT\")."
        );
        let default_seed_bytes: Vec<u8> = b"INIT".to_vec();
        let default_metadata: Box<dyn Any + Send + Sync> = Box::new(OnDiskCorpusEntryMetadata {
            source_description: "Default initial seed generated by Drifter CLI".to_string(),
        });
        corpus.add(default_seed_bytes, default_metadata)?;
    }

    let mut scheduler: Box<dyn Scheduler<Vec<u8>>> = Box::new(RandomScheduler::new());

    let mock_observer_name_str = MockCoverageObserver::new().name();
    let mut main_feedback: Box<dyn Feedback<Vec<u8>>> = Box::new(MockBitmapCoverageFeedback::new(
        mock_observer_name_str.to_string(),
    ));
    // let mut main_feedback: Box<dyn Feedback<Vec<u8>>> = Box::new(UniqueInputFeedback::new());

    let mut coverage_observer = MockCoverageObserver::new();
    // let mut noop_observer = NoOpObserver::default();

    let mut observers_for_executor: Vec<&mut dyn Observer> = vec![
        &mut coverage_observer,
        // &mut noop_observer,
    ];

    for i in 0..corpus.len() {
        if let Some((input_ref, _meta_ref)) = corpus.as_mut().get(i) {
            if let Some(mbf) = main_feedback
                .as_any_mut()
                .downcast_mut::<MockBitmapCoverageFeedback>()
            {
                let hash_of_seed = md5::compute(input_ref.as_bytes()).0;
                mbf.global_coverage_hashes.insert(hash_of_seed);
            } else if let Some(uif) = main_feedback
                .as_any_mut()
                .downcast_mut::<UniqueInputFeedback>()
            {
                uif.add_known_input_hash(input_ref);
            }
        }
    }
    main_feedback.init(corpus.as_ref())?;

    let max_iterations = drifter_config
        .fuzzer
        .as_ref()
        .map_or_else(drifter_core::config::default_iterations, |f_settings| {
            f_settings.max_iterations
        });
    println!(
        "INFO: Starting Drifter fuzz loop for {} iterations. Initial corpus size: {}.",
        max_iterations,
        corpus.len()
    );

    let fuzz_loop_start_time = Instant::now();
    let mut total_executions: u64 = 0;
    let mut solutions_found_count: u64 = 0;
    let mut last_log_time = Instant::now();

    for current_iteration in 0..max_iterations {
        let (corpus_input_id_selected, base_input_for_mutation_cloned) = match scheduler
            .next(corpus.as_mut(), &mut rng)
        {
            Ok(selected_id) => match corpus.as_mut().get(selected_id) {
                Some((input_ref, _meta_ref)) => (selected_id, input_ref.clone()),
                None => {
                    eprintln!(
                        "CRITICAL: Scheduler returned ID {} but corpus.get failed. Corpus inconsistency. Stopping.",
                        selected_id
                    );
                    break;
                }
            },
            Err(SchedulerError::CorpusEmpty) => {
                if corpus.is_empty() {
                    println!("WARN: Corpus empty. Attempting to generate a new seed from scratch.");
                    if let Ok(generated_seed) =
                        mutator.mutate(None, &mut rng, Some(corpus.as_ref()))
                    {
                        let emergency_meta: Box<dyn Any + Send + Sync> =
                            Box::new(OnDiskCorpusEntryMetadata {
                                source_description: "Emergency seed: corpus empty".to_string(),
                            });
                        match corpus.add(generated_seed.clone(), emergency_meta) {
                            Ok(new_id) => {
                                if let Some(mbf) = main_feedback
                                    .as_any_mut()
                                    .downcast_mut::<MockBitmapCoverageFeedback>()
                                {
                                    mbf.global_coverage_hashes
                                        .insert(md5::compute(generated_seed.as_bytes()).0);
                                } else if let Some(uif) = main_feedback
                                    .as_any_mut()
                                    .downcast_mut::<UniqueInputFeedback>()
                                {
                                    uif.add_known_input_hash(&generated_seed);
                                }
                                (new_id, generated_seed)
                            }
                            Err(e) => {
                                eprintln!(
                                    "ERROR: Failed to add emergency seed: {:?}. Stopping.",
                                    e
                                );
                                break;
                            }
                        }
                    } else {
                        eprintln!("ERROR: Mutator failed to generate seed from scratch. Stopping.");
                        break;
                    }
                } else {
                    eprintln!(
                        "WARN: Scheduler reported CorpusEmpty, but corpus.is_empty() is false. Skipping iteration."
                    );
                    continue;
                }
            }
            Err(e) => {
                eprintln!("FATAL: Scheduler error: {:?}. Stopping fuzz loop.", e);
                break;
            }
        };

        let mutated_input = match mutator.mutate(
            Some(&base_input_for_mutation_cloned),
            &mut rng,
            Some(corpus.as_ref()),
        ) {
            Ok(input) => input,
            Err(e) => {
                eprintln!(
                    "WARN: Mutation failed for input based on corpus ID {}: {:?}. Skipping.",
                    corpus_input_id_selected, e
                );
                continue;
            }
        };
        total_executions += 1;

        for obs in observers_for_executor.iter_mut() {
            obs.reset()
                .map_err(|e| anyhow::anyhow!("Observer '{}' reset failed: {}", obs.name(), e))?;
        }

        let execution_status = executor.execute_sync(&mutated_input, &mut observers_for_executor);

        let mut observer_data_map_for_feedback = HashMap::new();
        for obs_ref in observers_for_executor.iter() {
            observer_data_map_for_feedback.insert(obs_ref.name(), obs_ref.serialize_data());
        }

        let mut input_was_added_by_feedback = false;
        if main_feedback.is_interesting(
            &mutated_input,
            &observer_data_map_for_feedback,
            corpus.as_ref(),
        )? && main_feedback.add_to_corpus(mutated_input.clone(), corpus.as_mut())?
        {
            input_was_added_by_feedback = true;
        }

        let mut bug_found_this_iteration = false;
        if let Some(bug_report) = oracle.examine(&mutated_input, &execution_status, None) {
            solutions_found_count += 1;
            bug_found_this_iteration = true;
            println!(
                "\n!!! BUG FOUND (#{}) (Execution #{}) !!!",
                solutions_found_count, total_executions
            );
            println!("  Input Hash: {}", bug_report.input_hash);
            println!("  Description: {}", bug_report.description);
            if !input_was_added_by_feedback {
                let crash_metadata: Box<dyn Any + Send + Sync> =
                    Box::new(OnDiskCorpusEntryMetadata {
                        source_description: format!(
                            "Crash #{}: {} (Exec: {})",
                            solutions_found_count, bug_report.description, total_executions
                        ),
                    });
                match corpus.add(bug_report.input, crash_metadata) {
                    Ok(_) => println!("INFO: Crashing input added to corpus."),
                    Err(CorpusError::Io(e)) if e.contains("already exists") => {} // duplicate
                    Err(e) => eprintln!("ERROR: Could not add crashing input to corpus: {:?}", e),
                }
            }
        }

        scheduler.report_feedback(
            corpus_input_id_selected,
            &input_was_added_by_feedback,
            bug_found_this_iteration,
        );

        let now = Instant::now();
        if bug_found_this_iteration
            || input_was_added_by_feedback
            || now.duration_since(last_log_time) > Duration::from_secs(1)
        {
            let elapsed_seconds = fuzz_loop_start_time.elapsed().as_secs_f32();
            let exec_per_sec = if elapsed_seconds > 0.0 {
                total_executions as f32 / elapsed_seconds
            } else {
                0.0
            };
            print!(
                "\rIter: {:>5.0}k/{:<5.0}k, Execs: {:>8}, Corpus: {:>5}, Bugs: {:>3}, Time: {:>6.0}s, EPS: {:>7.1}   ",
                (current_iteration + 1) as f32 / 1000.0,
                max_iterations as f32 / 1000.0,
                total_executions,
                corpus.len(),
                solutions_found_count,
                elapsed_seconds,
                exec_per_sec
            );
            StdIoWrite::flush(&mut std::io::stdout()).unwrap();
            last_log_time = now;
        }
    }

    let total_fuzz_loop_duration = fuzz_loop_start_time.elapsed();
    println!("\n--- Fuzzing Campaign Summary ---");
    println!("Finished in: {:.2?}", total_fuzz_loop_duration);
    println!("Total Executions: {}", total_executions);
    println!("Final Corpus Size: {}", corpus.len());
    println!("Solutions Found: {}", solutions_found_count);
    if total_fuzz_loop_duration.as_secs_f32() > 0.0 {
        println!(
            "Average Executions per Second: {:.2}",
            total_executions as f32 / total_fuzz_loop_duration.as_secs_f32()
        );
    }
    Ok(())
}
