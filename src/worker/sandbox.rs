use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProfile {
    // Strict allowlist of commands only. No raw shell access.
    Safe,
    // Operations profile for limited admin access.
    Ops,
    // Full unfettered shell access (laboratory/local use only).
    Full,
}

impl SandboxProfile {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "safe" => Some(Self::Safe),
            "ops" => Some(Self::Ops),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

pub struct GlobalLimits {
    pub max_concurrent_jobs: usize,
    pub max_job_duration_sec: u64,
    pub max_output_bytes_total: usize,
}

impl Default for GlobalLimits {
    fn default() -> Self {
        Self {
            max_concurrent_jobs: 10,
            max_job_duration_sec: 3600, // 1 hour max
            max_output_bytes_total: 10 * 1024 * 1024, // 10MB default buffer limit to avoid OOM
        }
    }
}

pub struct ExecutionConfig {
    pub profile: SandboxProfile,
    pub limits: GlobalLimits,
    pub allowed_commands: Vec<String>,
    pub restricted_env: HashMap<String, String>,
    pub default_cwd: PathBuf,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            profile: SandboxProfile::Full,
            limits: GlobalLimits::default(),
            allowed_commands: vec![
                "echo".to_string(), 
                "docker".to_string(), 
                "python3".to_string(),
                "node".to_string()
            ],
            restricted_env: HashMap::new(),
            default_cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/tmp")),
        }
    }
}

pub struct SandboxManager {
    config: ExecutionConfig,
}

impl SandboxManager {
    pub fn new(config: ExecutionConfig) -> Self {
        Self { config }
    }

    pub fn validate_command(&self, command: &str) -> Result<()> {
        match self.config.profile {
            SandboxProfile::Safe => {
                if !self.config.allowed_commands.contains(&command.to_string()) {
                    bail!("Command '{}' is blocked by sandbox policy 'safe'", command);
                }
            }
            SandboxProfile::Ops => {
                // E.g., block dangerous commands like rm -rf, mkfs, etc.
                let blocked = ["rm", "dd", "mkfs", "fdisk", "reboot", "shutdown"];
                if blocked.contains(&command) {
                    bail!("Command '{}' is blocked by sandbox policy 'ops'", command);
                }
            }
            SandboxProfile::Full => {
                // Unrestricted
            }
        }
        Ok(())
    }

    pub fn get_limits(&self) -> &GlobalLimits {
        &self.config.limits
    }
}
