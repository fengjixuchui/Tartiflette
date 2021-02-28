//! Configuration

use crate::fuzz;
use clap::ArgMatches;
use std::{convert::TryFrom, fs, path::Path};

/// Persistent-binary signature - if found within file, it means it's a persistent mode binary
pub const PERSISTENT_SIG: &[u8] = b"\x01_LIBHFUZZ_PERSISTENT_BINARY_SIGNATURE_\x02\xFF";
/// HF NetDriver signature - if found within file, it means it's a NetDriver-based binary
pub const NETDRIVER_SIG: &[u8] = b"\x01_LIBHFUZZ_NETDRIVER_BINARY_SIGNATURE_\x02\xFF";
/// Max number of jobs
pub const MAX_JOBS: usize = 1024;

/// Error that can occured during the cli config parsing
#[derive(Debug)]
pub enum ConfigError {
    /// A configuration `field` is required
    Required(String),
    /// A `field` conversion error occured
    Conversion(String),
}

/// Config regarding I/O
#[derive(Debug)]
pub struct IOConfig {
    /// Input directory
    pub input_dir: String,
    /// Output directory
    pub output_dir: String,
    /// Crash directory
    pub crash_dir: String,
    /// Coverage directory
    pub cov_dir: String,
    /// Maximum file size
    pub max_file_size: usize,

    /// TODO
    pub input_file_count: usize,
}

impl IOConfig {
    /// Valide the `IOConfig`
    pub fn validate(&mut self) -> Result<(), String> {
        let input_dir = Path::new(&self.input_dir);
        if !input_dir.exists() {
            if let Err(error) = fs::create_dir(input_dir) {
                return Err(format!("{}", error));
            }
        }

        let output_dir = Path::new(&self.output_dir);
        if !output_dir.exists() {
            if let Err(error) = fs::create_dir(output_dir) {
                return Err(format!("{}", error));
            }
        }

        let crash_dir = Path::new(&self.crash_dir);
        if !crash_dir.exists() {
            if let Err(error) = fs::create_dir(crash_dir) {
                return Err(format!("{}", error));
            }
        }

        let cov_dir = Path::new(&self.cov_dir);
        if !cov_dir.exists() {
            if let Err(error) = fs::create_dir(cov_dir) {
                return Err(format!("{}", error));
            }
        }

        Ok(())
    }
}

impl TryFrom<&ArgMatches<'_>> for IOConfig {
    type Error = ConfigError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        let input_dir = matches
            .value_of("input")
            .map(String::from)
            .ok_or(ConfigError::Required("input".to_string()))?;
        let output_dir = matches
            .value_of("output")
            .map(String::from)
            .unwrap_or(input_dir.clone());
        let crash_dir = matches
            .value_of("crashdir")
            .map(String::from)
            .unwrap_or(input_dir.clone());
        let cov_dir = matches
            .value_of("crashdir")
            .map(String::from)
            .unwrap_or(input_dir.clone());
        let max_file_size = matches
            .value_of("max_file_size")
            .map(|s| s.parse::<usize>())
            .unwrap_or(Ok(128 * 1024 * 1024))
            .or(Err(ConfigError::Conversion("max_file_size".to_string())))?;

        Ok(Self {
            input_dir: input_dir,
            input_file_count: 0,
            output_dir: output_dir,
            crash_dir: crash_dir,
            cov_dir: cov_dir,
            max_file_size: max_file_size,
        })
    }
}

#[derive(Debug)]
pub struct ExeConfig {
    pub cmdline: Option<Vec<String>>,
    pub mutation_cmdline: Option<String>,
    pub post_mutation_cmdline: Option<String>,
    pub fb_mutation_cmdline: Option<String>,
}

impl ExeConfig {
    /// Valide the `ExeConfig`
    pub fn validate(&mut self) -> Result<(), String> {
        Ok(())
    }
}

impl TryFrom<&ArgMatches<'_>> for ExeConfig {
    type Error = ConfigError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        let cmdline = matches
            .values_of("program")
            .map(|vals| vals.map(String::from).collect::<Vec<_>>());

        Ok(Self {
            cmdline: cmdline,
            mutation_cmdline: matches.value_of("mutation_cmdline").map(String::from),
            post_mutation_cmdline: matches.value_of("post_mutation_cmdline").map(String::from),
            fb_mutation_cmdline: matches.value_of("fb_mutation_cmdline").map(String::from),
        })
    }
}

#[derive(Debug)]
pub struct AppConfig {
    pub verbose: u64,
    pub jobs: usize,
    pub minimize: bool,
    pub feedback_method: fuzz::FeedBackMethod,
    pub persistent: bool,
    pub netdriver: bool,
    pub crash_exit: bool,
    pub mutation_per_run: usize,
    pub mutation_num: Option<usize>,
    pub socket_fuzzer: bool,
    pub timeout: usize,
    pub max_input_size: usize,
    pub random_ascii: bool,
}

impl AppConfig {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.socket_fuzzer {
            self.timeout = 0;
        }

        if self.jobs == 0 {
            return Err(format!("Too few fuzzing threads specified"));
        }

        if self.jobs >= MAX_JOBS {
            return Err(format!(
                "Too many fuzzing threads specified {} >= {}",
                self.jobs, MAX_JOBS
            ));
        }

        Ok(())
    }
}

impl TryFrom<&ArgMatches<'_>> for AppConfig {
    type Error = ConfigError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        Ok(Self {
            verbose: matches.occurrences_of("verbose"),
            jobs: matches.value_of("jobs").unwrap().parse::<usize>().unwrap(),
            minimize: matches.is_present("minimize"),
            feedback_method: fuzz::FeedBackMethod::SOFT,
            persistent: matches.is_present("persistent"),
            netdriver: matches.is_present("netdriver"),
            crash_exit: matches.is_present("crash_exit"),
            mutation_per_run: matches
                .value_of("mutation_per_run")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            mutation_num: matches
                .value_of("mutation_num")
                .map(|val| val.parse::<usize>().unwrap()),
            socket_fuzzer: matches.is_present("socket_fuzzer"),
            timeout: matches
                .value_of("timeout")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            max_input_size: 0,
            random_ascii: matches.is_present("random_ascii"),
        })
    }
}

#[derive(Debug)]
// Global configuration
pub struct Config {
    /// I/O configuration
    pub io_config: IOConfig,
    /// Executable configuration
    pub exe_config: ExeConfig,
    /// Application config
    pub app_config: AppConfig,
}

impl Config {
    pub fn validate(&mut self) -> Result<(), String> {
        self.exe_config.validate();
        self.app_config.validate();
        self.io_config.validate()
    }
}

impl TryFrom<&ArgMatches<'_>> for Config {
    type Error = ConfigError;

    fn try_from(matches: &ArgMatches) -> Result<Self, Self::Error> {
        Ok(Self {
            io_config: IOConfig::try_from(matches)?,
            exe_config: ExeConfig::try_from(matches)?,
            app_config: AppConfig::try_from(matches)?,
        })
    }
}
