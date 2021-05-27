//! Fuzz system

use std::hint;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use std::{cmp, thread::sleep};

use tartiflette::vm::{Vm, VmExit};

use crate::app::App;
use crate::app::Mode;
use crate::config::Config;
use crate::corpus::{FuzzCov, FuzzInput};
use crate::feedback::FeedBackMethod;
use crate::input;
use crate::mangle;
use crate::random::Rand;
use crate::utils::log2;

#[derive(Debug)]
pub struct FuzzWorker {}

#[derive(Debug)]
pub struct FuzzCase {
    /// Unique id
    pub id: usize,
    /// Start run instant
    pub start_instant: Instant,

    /// Input data
    pub input: FuzzInput,

    /// Proccess id
    pub pid: Option<usize>,
    /// VM
    pub vm: Option<Vm>,

    pub static_file_try_more: bool,
    pub mutations_per_run: usize,
    pub tries: usize,

    /// Pseudo-random generator
    pub rand: Rand,
}

impl FuzzCase {
    pub fn new(app: &App) -> Self {
        let vm = {
            let exe = app.exe.lock().unwrap();
            exe.vm
                .as_ref()
                .unwrap()
                .fork(&exe.kvm.as_ref().unwrap())
                .unwrap()
        };

        Self {
            id: 0,
            start_instant: Instant::now(),

            pid: None,
            input: FuzzInput::new(app),
            vm: Some(vm),

            static_file_try_more: false,
            mutations_per_run: app.config.app_config.mutation_per_run,
            tries: 0,

            rand: Rand::new_random_seed(),
        }
    }

    pub fn set_input_size(&mut self, size: usize, config: &Config) {
        if self.input.data.len() == size {
            return;
        }

        if size > config.app_config.max_input_size {
            panic!(
                "Too large size requested: {} > {}",
                size, config.app_config.max_input_size
            );
        }
        self.input.data.resize(size, 0);
    }

    /// Run
    pub fn run(&mut self, app: &App) -> Result<(), ()> {
        self.id = app.metrics.fuzz_case_count.fetch_add(1, Ordering::Relaxed);
        let vm = self.vm.as_mut().unwrap();

        {
            let exe = app.exe.lock().unwrap();
            vm.reset(exe.vm.as_ref().unwrap()).unwrap();
        };

        vm.memory
            .write(
                0x80_000,
                &self.input.data[..self.input.data.len().min(0x1000)],
            )
            .unwrap();

        loop {
            let res = vm.run();

            if let VmExit::Exit = res.unwrap() {
                break;
            }
        }

        let elasped = self.start_instant.elapsed().as_millis() as usize;
        let mut max_elasped = app.metrics.max_fuzz_run_time_ms.lock().unwrap();
        *max_elasped = max_elasped.max(elasped);

        let coverage_count = vm.get_coverage().len();
        if coverage_count > 0 {
            println!("New coverage: {:x?}", coverage_count);

            {
                let mut feedback = app.feedback.lock().unwrap();
                feedback.breakpoint_count += coverage_count;
            }

            let mut cov_bytes = self.input.cov.bytes();
            cov_bytes[0] = 64 - log2(self.input.data.len()) as usize;


            {
                let corpus = app.corpus.lock().unwrap();
                let file_name = self.input.generate_filename();

                if !corpus.contains(&file_name) {
                    core::mem::drop(corpus);
                    add_dynamic_input(self, app);
                }

            }
        }

        // assert!(vm.get_coverage().len() == 1);

        Ok(())
    }
}

fn write_cov_file(dir: &str, file: &FuzzInput) {
    let file_name = file.generate_filename();
    let file_path_name = format!("{}/{}", dir, file_name);
    let file_path = std::path::Path::new(&file_path_name);

    if file_path.exists() {
        println!(
            "File {} already exists in the output corpus directory",
            file_name
        );
        return;
        todo!();
    }

    println!("Adding file {} to the corpus directory {}", file_name, dir);
    println!("Written {} bytes to {:?}", file.data.len(), file_path);
    std::fs::write(file_path, &file.data[..file.data.len()]).unwrap();
}

fn add_dynamic_input(case: &mut FuzzCase, app: &App) {
    app.metrics.last_cov_update.store(
        app.metrics.start_instant.elapsed().as_secs() as usize,
        Ordering::Relaxed,
    );
    let fuzz_file = case
        .input
        .fork(case.start_instant.elapsed().as_millis() as usize, app);

    // Max coverage
    {
        let mut max_cov = app.max_cov.lock().unwrap();
        *max_cov = max_cov.compute_local_max(&fuzz_file.cov);
    }

    // Max fuzz file size
    {
        let mut max_size = app.metrics.fuzz_input_max_size.lock().unwrap();
        *max_size = cmp::max(*max_size, fuzz_file.data.len());
    }

    if !app.config.app_config.socket_fuzzer {
        write_cov_file(&app.config.io_config.output_dir, &fuzz_file);
    }

    {
        let mut corpus = app.corpus.lock().unwrap();
        corpus.add_file(fuzz_file);
    }

    if app.config.app_config.socket_fuzzer {
        // Din't add coverage data to files in socket fuzzer mode
        return;
    }

    // No need to add files to the new coverage dir, if it's not the main phase
    if app.get_mode() != Mode::DynamicMain {
        return;
    }

    app.metrics.new_units_added.fetch_add(1, Ordering::Relaxed);

    if false {
        todo!("covdir new");
    }
}

fn set_dynamic_main_state(case: &mut FuzzCase, app: &App) {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    COUNT.fetch_add(1, Ordering::Relaxed);

    // TODO let _lock = app.mutex.lock().unwrap();

    if app.get_mode() != Mode::DynamicDryRun {
        // Already switched out of the Dry Run
        return;
    }

    println!("Entering phase 2/3: Switching to the feedback driven mode.");
    app.switching_feedback.store(true, Ordering::Relaxed);

    loop {
        if COUNT.load(Ordering::Relaxed) == app.config.app_config.jobs {
            break;
        }
        if app.is_terminating() {
            return;
        }

        thread::sleep(std::time::Duration::from_millis(10));
        hint::spin_loop();
    }
    app.switching_feedback.store(false, Ordering::Relaxed);

    if app.config.app_config.minimize {
        println!("Entering phase 3/3: Coprus minimization");
        app.set_mode(Mode::DynamicMinimize);
        return;
    }

    /*
     * If the initial fuzzing yielded no useful coverage, just add a single empty file to the
     * dynamic corpus, so the dynamic phase doesn't fail because of lack of useful inputs
     */
    if app.metrics.fuzz_input_count.load(Ordering::Relaxed) == 0 {
        let mut fuzz_file = FuzzInput::default();
        fuzz_file.filename = "[DYNAMIC-0-SIZE]".to_string();
        core::mem::swap(&mut fuzz_file, &mut case.input);
        println!("Empty file!");
        add_dynamic_input(case, app);
        core::mem::swap(&mut fuzz_file, &mut case.input);
    }
    case.input.filename = "[DYNAMIC]".to_string();

    if app.config.io_config.max_file_size == 0
        && app.config.app_config.max_input_size > input::INPUT_MIN_SIZE
    {
        let mut new_size = cmp::max(
            *app.metrics.fuzz_input_max_size.lock().unwrap(),
            input::INPUT_MIN_SIZE,
        );
        new_size = cmp::min(new_size, app.config.app_config.max_input_size);
        println!(
            "Setting maximum input size to {} bytes, previously: {}",
            new_size, app.config.app_config.max_input_size
        );
        panic!();
    }

    println!("Entering phase 3/3: Dynamic Main (Feedback driven Mode)");
    app.set_mode(Mode::DynamicMain);
}

fn minimize_remove_files(case: &mut FuzzCase) {
    panic!();
}

fn input_should_read_new_file(app: &App, case: &mut FuzzCase) -> bool {
    if app.get_mode() != Mode::DynamicDryRun {
        case.set_input_size(app.config.app_config.max_input_size, &app.config);

        return true;
    }

    if !case.static_file_try_more {
        case.static_file_try_more = true;
        // Start with 4 bytes, increase the size in following iterations
        case.set_input_size(
            std::cmp::min(4, app.config.app_config.max_input_size),
            &app.config,
        );
        println!("{}", case.input.data.len());

        return true;
    }

    let new_size = std::cmp::max(
        case.input.data.len() * 2,
        app.config.app_config.max_input_size,
    );
    if new_size == app.config.app_config.max_input_size {
        case.static_file_try_more = false;
    }

    case.set_input_size(new_size, &app.config);
    false
}

fn fuzz_prepare_static_file(app: &App, case: &mut FuzzCase, mangle: bool) -> bool {
    let mut ent = None;

    if input_should_read_new_file(&app, case) {
        for entry in app.input.entries() {
            println!("{:?}", entry);
            ent = Some(entry.clone());

            if !mangle {
                let corpus = app.corpus.lock().unwrap();
                if corpus.contains(&entry) {
                    eprintln!("Skipping {}, as it's already in the dynamic corpus", &entry);
                    break;
                }
            }
            app.metrics
                .tested_file_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    if ent.is_none() {
        return false;
    }

    let pathname = app.input.get_path_to(ent.as_ref().unwrap());
    let mut file = std::fs::File::open(pathname).unwrap();
    case.input.data = vec![0; case.input.data.len()];
    let size = file.read(&mut case.input.data).unwrap();
    println!(
        "Read {} bytes / {} from {:?}",
        size,
        case.input.data.len(),
        ent.as_ref().unwrap()
    );

    if case.static_file_try_more && size < case.input.data.len() {
        // The file is smaller than the requested size, no need to reread it anymore
        case.static_file_try_more = false;
    }
    case.set_input_size(size, &app.config);
    case.input.cov = FuzzCov::default();
    case.input.idx = 0;
    case.input.refs = 0;

    if mangle {
        mangle::mangle_content(case, 0, app);
    }

    return true;
}

fn input_speed_factor(app: &App, case: &mut FuzzCase) -> isize {
    // Slower the input, lower the chance of it being tested
    let mut avg_usecs_per_input = app.metrics.start_instant.elapsed().as_micros() as usize;
    avg_usecs_per_input /= app.metrics.mutations_count.load(Ordering::Relaxed);
    avg_usecs_per_input /= app.config.app_config.jobs;
    avg_usecs_per_input = avg_usecs_per_input.clamp(1, 1000000);

    let mut sample_usecs = case
        .start_instant
        .saturating_duration_since(app.metrics.start_instant)
        .as_micros() as usize;
    sample_usecs = sample_usecs.clamp(1, 1000000);

    match sample_usecs >= avg_usecs_per_input {
        true => (sample_usecs / avg_usecs_per_input) as isize,

        false => -((avg_usecs_per_input / sample_usecs) as isize),
    }
}

fn input_skip_factor(app: &App, case: &mut FuzzCase, file: &FuzzInput) -> (isize, isize) {
    let mut penalty: isize = 0;
    let speed_factor = input_speed_factor(app, case).clamp(-10, 2);
    penalty += speed_factor;

    /* Older inputs -> lower chance of being tested */
    let percentile = (file.idx * 100) / app.metrics.fuzz_input_count.load(Ordering::Relaxed);
    if percentile <= 40 {
        penalty += 2;
    } else if percentile <= 70 {
        penalty += 1;
    } else if percentile <= 80 {
        penalty += 0;
    } else if percentile <= 90 {
        penalty += -1;
    } else if percentile <= 97 {
        penalty += -2;
    } else if percentile <= 199 {
        penalty += -3;
    } else {
        panic!();
    }

    /* Add penalty for the input being too big - 0 is for 1kB inputs */
    if file.data.len() > 0 {
        let mut bias = ((core::mem::size_of::<isize>() * 8) as u32
            - file.data.len().leading_zeros()
            - 1) as isize;
        bias -= 10;
        bias = bias.clamp(-5, 5);
        penalty += bias;
    }

    (speed_factor, penalty)
}

fn prepare_dynamic_input(app: &App, case: &mut FuzzCase, mangle: bool) -> bool {
    if app.metrics.fuzz_input_count.load(Ordering::Relaxed) == 0 {
        unreachable!();
    }
    let corpus = app.corpus.lock().unwrap();
    let mut files = match *app.current_file.lock().unwrap() {
        Some(ref path) => corpus.iter_from(path),
        None => corpus.iter(),
    };
    let mut speed_factor = 0;
    let mut file = files.next().unwrap();

    loop {
        if case.tries > 0 {
            case.tries -= 1;
            break;
        }

        let (a, b) = input_skip_factor(app, case, &file);
        speed_factor = a;

        let skip_factor = b;
        if skip_factor <= 0 {
            case.tries = (-skip_factor) as usize;
            break;
        }

        if case.rand.next() % skip_factor as u64 == 0 {
            break;
        }

        file = match files.next() {
            Some(file) => file,
            None => {
                files = corpus.iter();
                files.next().unwrap()
            }
        };
    }
    *app.current_file.lock().unwrap() = files.next().map(|file| file.filename.clone());

    case.set_input_size(file.data.len(), &app.config);
    case.input.idx = file.idx;
    case.input.exec_usec = file.exec_usec;
    //case.input.src = file;
    case.input.refs = 0;
    case.input.cov = file.cov;
    case.input.filename = file.filename.clone();
    case.input.data = file.data.clone();

    core::mem::drop(corpus);

    if mangle {
        mangle::mangle_content(case, speed_factor, app)
    }

    true
}

fn fuzz_fetch_input(app: &App, case: &mut FuzzCase) -> bool {
    if app.get_mode() == Mode::DynamicDryRun {
        case.mutations_per_run = 0;
        if fuzz_prepare_static_file(app, case, true) {
            return true;
        }
        set_dynamic_main_state(case, app);
        case.mutations_per_run = app.config.app_config.mutation_per_run;
    }

    if app.get_mode() == Mode::DynamicMinimize {
        minimize_remove_files(case);
        return false;
    }

    if app.get_mode() == Mode::DynamicMain {
        if app.config.exe_config.mutation_cmdline.is_some() {
            todo!();
        } else if app.config.exe_config.fb_mutation_cmdline.is_some() {
            if !prepare_dynamic_input(app, case, false) {
                eprintln!("Failed");
                return false;
            }
        } else {
            if !prepare_dynamic_input(app, case, true) {
                eprintln!("Failed");
                return false;
            }
        }
    }

    if app.get_mode() == Mode::Static {
        todo!();
    }

    return true;
}

fn compute_feedback(app: &App, case: &mut FuzzCase) {}

fn report_save_report(app: &App, case: &mut FuzzCase) {}

fn fuzz_loop(app: &App, case: &mut FuzzCase) {
    case.mutations_per_run = app.config.app_config.mutation_per_run;

    if !fuzz_fetch_input(app, case) {
        if app.config.app_config.minimize && app.get_mode() == Mode::DynamicMinimize {
            app.set_terminating();
            return;
        }
        eprintln!("Could not prepare input for fuzzing");
    }

    if case.run(&app).is_err() {
        eprintln!("Couldn't run fuzzed command");
    }

    if app.config.app_config.feedback_method != FeedBackMethod::NONE {
        compute_feedback(app, case);
    }

    report_save_report(app, case);
}

/// Fuzz worker
pub fn worker(app: Arc<App>, id: usize) {
    app.metrics.job_active_count.fetch_add(1, Ordering::Relaxed);
    println!("Launched fuzzing threads: no {}", id);

    let mapname = format!("tf-{}-input", id);
    println!("{}", mapname);
    let mut case = FuzzCase::new(&app);

    let mapname = format!("tf-{}-perthreadmap", id);
    println!("{}", mapname);

    loop {
        let mutation_count = app.metrics.mutations_count.fetch_add(1, Ordering::Relaxed);

        if let Some(mutation_num) = app.config.app_config.mutation_num {
            if mutation_count >= mutation_num {
                break;
            }
        }

        fuzz_loop(&app, &mut case);
        if app.is_terminating() {
            break;
        }

        if app.config.app_config.crash_exit {
            if app.metrics.crashes_count.load(Ordering::Relaxed) > 0 {
                println!("Global crash");
                app.set_terminating();
                break;
            }
        }
    }

    println!("Terminated fuzzing threads: no {}", id);
    app.metrics
        .job_finished_count
        .fetch_add(1, Ordering::Relaxed);
}

/// Compute the starting fuzz mode based on the config
fn compute_fuzz_mode(config: &Config) -> Mode {
    let mode = if config.app_config.socket_fuzzer {
        Mode::DynamicMain
    } else if config.app_config.feedback_method != FeedBackMethod::NONE {
        Mode::DynamicDryRun
    } else {
        Mode::Static
    };

    // Log mode
    match mode {
        Mode::DynamicMain => {
            println!("Entering phase - Feedbaclk drvier mode (SocketFuzzer)");
        }
        Mode::DynamicDryRun => {
            println!("Entering phase 1/3: Dry run");
        }
        Mode::Static => {
            println!("Entering phase: Static");
        }
        _ => unreachable!(),
    }

    mode
}

pub fn supervisor(app: Arc<App>) {
    // Start a timer
    let start = Instant::now();

    let mut last_cases = 0;
    let mut last_time = Instant::now();
    loop {
        let delta = start.elapsed().as_secs_f64();
        let last_delta = last_time.elapsed().as_secs_f64();

        let fuzz_cases = app.metrics.fuzz_case_count.load(Ordering::Relaxed);
        eprintln!(
            "[{:8.4}] Execs {:8} | exec/s {:8.0}",
            delta,
            fuzz_cases,
            (fuzz_cases - last_cases) as f64 / last_delta,
        );
        last_cases = fuzz_cases;
        last_time = Instant::now();

        if app.is_terminating() {
            break;
        }

        sleep(std::time::Duration::from_millis(500));
    }
}

/// Start fuzzing
pub fn fuzz(config: Config) {
    let mut threads = Vec::new();

    // Create the App
    let mode = compute_fuzz_mode(&config);
    let app = Arc::new(App::new(config, mode));
    println!("{:#?}", app);

    // Loop through numbers of jobs
    for i in 0..app.config.app_config.jobs {
        // Create a thread builder
        let builder = thread::Builder::new()
            .stack_size(1024 * 1024)
            .name(format!("fuzz_worker({})", i));

        // Arc clone the App
        let app = Arc::clone(&app);

        // Launch the thread
        let thread = builder
            .spawn(move || {
                worker(app, i);
            })
            .unwrap();
        threads.push(thread);
    }

    // Launch the supervisor
    let builder = thread::Builder::new()
        .stack_size(1024 * 1024)
        .name("fuzz_supervisor".to_string());

    // Arc clone the App
    let app = Arc::clone(&app);

    // Launch the thread
    let thread = builder
        .spawn(move || {
            supervisor(Arc::clone(&app));
        })
        .unwrap();
    threads.push(thread);

    // Wait for thread completion
    for thread in threads {
        thread.join().unwrap();
    }
}