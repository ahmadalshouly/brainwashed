use serde::{Deserialize, Serialize};

/// Which llama.cpp build to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Pick the best build for this machine.
    #[default]
    Auto,
    Cpu,
    /// Apple GPUs. The macOS arm64 build always includes it.
    Metal,
    /// Most AMD, Intel and NVIDIA GPUs on Windows and Linux.
    Vulkan,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hardware {
    pub os: &'static str,
    pub arch: &'static str,
    pub total_memory_bytes: u64,
    pub cpu_cores: usize,
}

impl Hardware {
    pub fn detect() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        Hardware {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            total_memory_bytes: sys.total_memory(),
            cpu_cores: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        }
    }

    /// Resolves `Auto` to a concrete backend for this machine.
    pub fn resolve(&self, backend: Backend) -> Backend {
        match backend {
            Backend::Auto => match (self.os, self.arch) {
                ("macos", "aarch64") => Backend::Metal,
                ("windows" | "linux", "x86_64") => Backend::Vulkan,
                _ => Backend::Cpu,
            },
            other => other,
        }
    }

    /// The largest model size class (in billions of parameters, Q4) that fits
    /// comfortably next to the OS and other apps.
    pub fn recommended_max_params_b(&self) -> u32 {
        let gb = self.total_memory_bytes / (1024 * 1024 * 1024);
        match gb {
            0..=7 => 1,
            8..=15 => 3,
            16..=31 => 8,
            _ => 14,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(os: &'static str, arch: &'static str, gb: u64) -> Hardware {
        Hardware {
            os,
            arch,
            total_memory_bytes: gb * 1024 * 1024 * 1024,
            cpu_cores: 8,
        }
    }

    #[test]
    fn auto_backend_per_platform() {
        assert_eq!(
            hw("macos", "aarch64", 16).resolve(Backend::Auto),
            Backend::Metal
        );
        assert_eq!(
            hw("macos", "x86_64", 16).resolve(Backend::Auto),
            Backend::Cpu
        );
        assert_eq!(
            hw("windows", "x86_64", 16).resolve(Backend::Auto),
            Backend::Vulkan
        );
        assert_eq!(
            hw("linux", "aarch64", 16).resolve(Backend::Auto),
            Backend::Cpu
        );
        assert_eq!(
            hw("linux", "x86_64", 16).resolve(Backend::Cpu),
            Backend::Cpu
        );
    }

    #[test]
    fn recommends_by_memory() {
        assert_eq!(hw("linux", "x86_64", 8).recommended_max_params_b(), 3);
        assert_eq!(hw("linux", "x86_64", 32).recommended_max_params_b(), 14);
    }
}
