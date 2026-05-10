//! Nyquest Preflight — Full system requirements check
//!
//! Validates hardware, OS, dependencies, GPU, and semantic stage
//! readiness before install or deploy. Maps to tiers from the
//! System Requirements document.

use console::style;
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

// ── Tier Definitions ──

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tier {
    Tier1RulesOnly,
    Tier2GpuSemantic,
    Tier3CpuSemantic,
    Insufficient,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Tier1RulesOnly => write!(f, "Tier 1 — Rules Only (no GPU)"),
            Tier::Tier2GpuSemantic => write!(f, "Tier 2 — GPU Semantic (full pipeline)"),
            Tier::Tier3CpuSemantic => write!(f, "Tier 3 — CPU Semantic (Ollama on CPU)"),
            Tier::Insufficient => write!(f, "Below minimum requirements"),
        }
    }
}

// ── Check Result ──

#[derive(Clone)]
struct CheckResult {
    category: &'static str,
    label: String,
    status: CheckStatus,
    detail: Option<String>,
    tier_impact: Option<String>,
}

#[derive(Clone, PartialEq)]
enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Info,
}

// ── System Info Collectors ──

fn get_cpu_cores() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .map(|s| s.lines().filter(|l| l.starts_with("processor")).count())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("sysctl")
            .args(["-n", "hw.ncpu"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn get_total_ram_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| {
                    l.split_whitespace()
                        .nth(1)?
                        .parse::<u64>()
                        .ok()
                        .map(|kb| kb / 1024)
                })
        })
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .map(|b| b / 1024 / 1024)
            })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn get_available_ram_mb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemAvailable:"))
                .and_then(|l| {
                    l.split_whitespace()
                        .nth(1)?
                        .parse::<u64>()
                        .ok()
                        .map(|kb| kb / 1024)
                })
        })
    }
    #[cfg(target_os = "macos")]
    {
        // macOS doesn't have MemAvailable, approximate from vm_stat
        None
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn get_disk_free_mb(path: &str) -> Option<u64> {
    Command::new("df")
        .args(["-m", path])
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.lines()
                .nth(1)
                .and_then(|l| l.split_whitespace().nth(3)?.parse::<u64>().ok())
        })
}

fn get_os_info() -> String {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
            })
            .unwrap_or_else(|| "Linux (unknown distro)".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("sw_vers")
            .args(["-productVersion"])
            .output()
            .ok()
            .map(|o| format!("macOS {}", String::from_utf8_lossy(&o.stdout).trim()))
            .unwrap_or_else(|| "macOS".to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "Unknown OS".to_string()
    }
}

fn get_arch() -> String {
    std::env::consts::ARCH.to_string()
}

fn get_glibc_version() -> Option<String> {
    Command::new("ldd")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            let err = String::from_utf8_lossy(&o.stderr);
            let combined = format!("{}{}", out, err);
            combined.lines().next().and_then(|l| {
                // Parse "ldd (Ubuntu GLIBC 2.39-0ubuntu6.3) 2.39" or similar
                l.rsplit(' ').next().map(|s| s.trim().to_string())
            })
        })
}

// ── GPU Detection ──

#[derive(Debug, Clone)]
struct GpuInfo {
    name: String,
    vram_total_mb: u64,
    vram_used_mb: u64,
    driver_version: String,
    cuda_version: Option<String>,
}

fn detect_nvidia_gpu() -> Option<Vec<GpuInfo>> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.used,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let out = String::from_utf8_lossy(&output.stdout);
    let mut gpus = Vec::new();

    for line in out.lines() {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() >= 4 {
            gpus.push(GpuInfo {
                name: parts[0].to_string(),
                vram_total_mb: parts[1].parse().unwrap_or(0),
                vram_used_mb: parts[2].parse().unwrap_or(0),
                driver_version: parts[3].to_string(),
                cuda_version: get_cuda_version(),
            });
        }
    }

    if gpus.is_empty() {
        None
    } else {
        Some(gpus)
    }
}

fn get_cuda_version() -> Option<String> {
    Command::new("nvidia-smi").output().ok().and_then(|o| {
        let out = String::from_utf8_lossy(&o.stdout);
        out.lines()
            .find(|l| l.contains("CUDA Version"))
            .and_then(|l| {
                l.split("CUDA Version:")
                    .nth(1)
                    .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
            })
    })
}

fn detect_apple_gpu() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("system_profiler")
            .args(["SPDisplaysDataType"])
            .output()
            .ok()
            .and_then(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                out.lines()
                    .find(|l| l.contains("Chipset Model:") || l.contains("Chip:"))
                    .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string())
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

// ── Ollama Detection ──

fn check_ollama_installed() -> Option<String> {
    Command::new("ollama")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                // Some versions output to stderr
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if err.contains("ollama") {
                    Some(err)
                } else {
                    None
                }
            }
        })
}

fn check_ollama_running() -> bool {
    TcpStream::connect_timeout(&"127.0.0.1:11434".parse().unwrap(), Duration::from_secs(2)).is_ok()
}

fn check_ollama_model(model: &str) -> bool {
    Command::new("ollama")
        .arg("list")
        .output()
        .ok()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            out.contains(model) || out.contains(&model.replace(":", "/"))
        })
        .unwrap_or(false)
}

fn check_ollama_health(endpoint: &str) -> bool {
    // Quick HTTP check against the Ollama API
    let base = endpoint
        .trim_end_matches("/v1/chat/completions")
        .trim_end_matches('/');

    Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--connect-timeout",
            "2",
            base,
        ])
        .output()
        .ok()
        .map(|o| {
            let code = String::from_utf8_lossy(&o.stdout).trim().to_string();
            code == "200"
        })
        .unwrap_or(false)
}

// ── Port Check ──

fn check_port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("0.0.0.0", port)).is_ok()
}

fn check_port_responding(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        Duration::from_secs(2),
    )
    .is_ok()
}

// ── Main Preflight Runner ──

#[allow(unused_variables, unused_mut, unused_assignments)]
pub fn run_preflight(config_path: &str, verbose: bool) {
    let mut checks: Vec<CheckResult> = Vec::new();
    let mut recommended_tier = Tier::Tier1RulesOnly;

    // ═══════════════════════════════════════════
    // CATEGORY: Operating System
    // ═══════════════════════════════════════════

    let os_info = get_os_info();
    let arch = get_arch();
    checks.push(CheckResult {
        category: "OS",
        label: format!("{} ({})", os_info, arch),
        status: CheckStatus::Info,
        detail: None,
        tier_impact: None,
    });

    // Architecture check
    match arch.as_str() {
        "x86_64" | "aarch64" => {
            checks.push(CheckResult {
                category: "OS",
                label: format!("Architecture: {} — supported", arch),
                status: CheckStatus::Pass,
                detail: None,
                tier_impact: None,
            });
        }
        _ => {
            checks.push(CheckResult {
                category: "OS",
                label: format!("Architecture: {} — untested", arch),
                status: CheckStatus::Warn,
                detail: Some("Nyquest is built for x86_64 and aarch64. Other architectures may work but are untested.".into()),
                tier_impact: None,
            });
        }
    }

    // glibc (Linux only)
    #[cfg(target_os = "linux")]
    {
        if let Some(glibc) = get_glibc_version() {
            let ver_parts: Vec<u32> = glibc.split('.').filter_map(|s| s.parse().ok()).collect();
            let ok = ver_parts.len() >= 2
                && (ver_parts[0] > 2 || (ver_parts[0] == 2 && ver_parts[1] >= 31));
            checks.push(CheckResult {
                category: "OS",
                label: format!("glibc {}", glibc),
                status: if ok {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                detail: if ok {
                    None
                } else {
                    Some("Requires glibc >= 2.31 (Ubuntu 20.04+, Debian 11+, RHEL 8+)".into())
                },
                tier_impact: if ok {
                    None
                } else {
                    Some("Binary will not run".into())
                },
            });
            if !ok {
                recommended_tier = Tier::Insufficient;
            }
        }
    }

    // ═══════════════════════════════════════════
    // CATEGORY: CPU
    // ═══════════════════════════════════════════

    if let Some(cores) = get_cpu_cores() {
        let (status, detail) = if cores >= 4 {
            (CheckStatus::Pass, None)
        } else if cores >= 2 {
            (
                CheckStatus::Pass,
                Some("Minimum met (2 cores). 4+ recommended for concurrent load.".into()),
            )
        } else {
            (
                CheckStatus::Fail,
                Some("Nyquest requires at least 2 CPU cores.".into()),
            )
        };
        checks.push(CheckResult {
            category: "CPU",
            label: format!("{} CPU cores", cores),
            status,
            detail,
            tier_impact: if cores < 2 {
                Some("Insufficient for any tier".into())
            } else {
                None
            },
        });
        if cores < 2 {
            recommended_tier = Tier::Insufficient;
        }
    }

    // ═══════════════════════════════════════════
    // CATEGORY: Memory
    // ═══════════════════════════════════════════

    if let Some(total_mb) = get_total_ram_mb() {
        let total_gb = total_mb as f64 / 1024.0;
        let (status, detail, tier) = if total_mb >= 6144 {
            (
                CheckStatus::Pass,
                Some(format!(
                    "{:.1} GB — sufficient for Tier 2 (GPU semantic) or Tier 3 (CPU semantic)",
                    total_gb
                )),
                if total_mb >= 8192 {
                    None
                } else {
                    Some(
                        "Tier 2 OK, Tier 3 may be tight (8 GB recommended for CPU semantic)".into(),
                    )
                },
            )
        } else if total_mb >= 2048 {
            (
                CheckStatus::Warn,
                Some(format!(
                    "{:.1} GB — sufficient for Tier 1 (rules only). Semantic stage requires 6+ GB.",
                    total_gb
                )),
                Some("Tier 1 only".into()),
            )
        } else if total_mb >= 512 {
            (
                CheckStatus::Pass,
                Some(format!(
                    "{:.1} GB — minimum met for Tier 1 (rules only)",
                    total_gb
                )),
                Some("Tier 1 only — rule engine uses ~71 MB RSS".into()),
            )
        } else {
            (
                CheckStatus::Fail,
                Some(format!(
                    "{:.1} GB — below minimum (512 MB required)",
                    total_gb
                )),
                Some("Insufficient".into()),
            )
        };

        checks.push(CheckResult {
            category: "Memory",
            label: format!("{:.1} GB total RAM", total_gb),
            status,
            detail,
            tier_impact: tier,
        });

        if total_mb < 512 {
            recommended_tier = Tier::Insufficient;
        } else if total_mb >= 6144 && recommended_tier == Tier::Tier1RulesOnly {
            // Enough RAM for semantic, but need GPU check to determine tier 2 vs 3
            recommended_tier = Tier::Tier3CpuSemantic;
        }
    }

    if let Some(avail_mb) = get_available_ram_mb() {
        let avail_gb = avail_mb as f64 / 1024.0;
        let status = if avail_mb >= 2048 {
            CheckStatus::Pass
        } else if avail_mb >= 512 {
            CheckStatus::Warn
        } else {
            CheckStatus::Fail
        };
        checks.push(CheckResult {
            category: "Memory",
            label: format!("{:.1} GB available RAM", avail_gb),
            status,
            detail: if avail_mb < 512 {
                Some("Low available memory. Close other applications or add RAM.".into())
            } else {
                None
            },
            tier_impact: None,
        });
    }

    // ═══════════════════════════════════════════
    // CATEGORY: Disk
    // ═══════════════════════════════════════════

    let install_path = Path::new(config_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    if let Some(free_mb) = get_disk_free_mb(&install_path) {
        let free_gb = free_mb as f64 / 1024.0;
        // Binary: 6.3 MB, Ollama + model: ~1.5 GB, logs: variable
        let (status, detail) = if free_mb >= 5120 {
            (
                CheckStatus::Pass,
                Some(format!(
                    "{:.1} GB free — plenty for binary + Ollama + model + logs",
                    free_gb
                )),
            )
        } else if free_mb >= 2048 {
            (
                CheckStatus::Pass,
                Some(format!(
                    "{:.1} GB free — sufficient (model ~1.2 GB, binary ~6 MB)",
                    free_gb
                )),
            )
        } else if free_mb >= 100 {
            (
                CheckStatus::Warn,
                Some(format!(
                    "{:.1} GB free — tight. Semantic model requires ~1.2 GB disk.",
                    free_gb
                )),
            )
        } else {
            (
                CheckStatus::Fail,
                Some(format!("{:.0} MB free — insufficient", free_mb as f64)),
            )
        };
        checks.push(CheckResult {
            category: "Disk",
            label: format!("{:.1} GB free on {}", free_gb, install_path),
            status,
            detail,
            tier_impact: if free_mb < 100 {
                Some("Cannot install".into())
            } else {
                None
            },
        });
    }

    // ═══════════════════════════════════════════
    // CATEGORY: GPU
    // ═══════════════════════════════════════════

    let mut has_gpu = false;
    let mut gpu_vram_mb: u64 = 0;

    if let Some(gpus) = detect_nvidia_gpu() {
        has_gpu = true;
        for gpu in &gpus {
            gpu_vram_mb = gpu.vram_total_mb;
            let vram_free = gpu.vram_total_mb.saturating_sub(gpu.vram_used_mb);

            let (status, detail) = if gpu.vram_total_mb >= 2048 {
                (
                    CheckStatus::Pass,
                    Some(format!(
                        "{} MB total, {} MB free — Qwen 2.5 1.5B needs ~1.5 GB VRAM",
                        gpu.vram_total_mb, vram_free
                    )),
                )
            } else {
                (CheckStatus::Warn,
                 Some(format!("{} MB VRAM — below 2 GB minimum for GPU semantic. CPU fallback will be used.",
                     gpu.vram_total_mb)))
            };

            checks.push(CheckResult {
                category: "GPU",
                label: format!("NVIDIA {} ({} MB VRAM)", gpu.name, gpu.vram_total_mb),
                status,
                detail,
                tier_impact: if gpu.vram_total_mb >= 2048 {
                    Some("Tier 2 — GPU semantic (200-350ms latency)".into())
                } else {
                    Some("Tier 3 — CPU semantic fallback".into())
                },
            });

            // Driver version
            let drv_parts: Vec<u32> = gpu
                .driver_version
                .split('.')
                .filter_map(|s| s.parse().ok())
                .collect();
            let drv_ok = drv_parts.first().copied().unwrap_or(0) >= 525;
            checks.push(CheckResult {
                category: "GPU",
                label: format!("NVIDIA driver {}", gpu.driver_version),
                status: if drv_ok {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Warn
                },
                detail: if drv_ok {
                    None
                } else {
                    Some("Driver 525+ recommended for Ollama GPU support".into())
                },
                tier_impact: None,
            });

            if let Some(ref cuda) = gpu.cuda_version {
                checks.push(CheckResult {
                    category: "GPU",
                    label: format!("CUDA {}", cuda),
                    status: CheckStatus::Pass,
                    detail: None,
                    tier_impact: None,
                });
            }
        }

        if gpu_vram_mb >= 2048 && recommended_tier != Tier::Insufficient {
            recommended_tier = Tier::Tier2GpuSemantic;
        }
    } else if let Some(apple_gpu) = detect_apple_gpu() {
        has_gpu = true;
        checks.push(CheckResult {
            category: "GPU",
            label: format!("Apple {} (Metal/unified memory)", apple_gpu),
            status: CheckStatus::Pass,
            detail: Some("Apple Silicon uses unified memory for GPU compute. Ollama supports Metal acceleration natively.".into()),
            tier_impact: Some("Tier 2 equivalent — GPU semantic via Metal".into()),
        });
        if recommended_tier != Tier::Insufficient {
            recommended_tier = Tier::Tier2GpuSemantic;
        }
    } else {
        checks.push(CheckResult {
            category: "GPU",
            label: "No NVIDIA GPU detected".into(),
            status: CheckStatus::Info,
            detail: Some("GPU not required for Tier 1 (rules only). Semantic stage will use CPU fallback (1-4s latency).".into()),
            tier_impact: Some("Tier 1 or Tier 3 (CPU semantic)".into()),
        });
    }

    // ═══════════════════════════════════════════
    // CATEGORY: Dependencies
    // ═══════════════════════════════════════════

    // Rust
    if let Ok(output) = Command::new("rustc").arg("--version").output() {
        let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
        checks.push(CheckResult {
            category: "Dependencies",
            label: format!("Rust: {}", ver),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else {
        checks.push(CheckResult {
            category: "Dependencies",
            label: "Rust: not installed".into(),
            status: CheckStatus::Warn,
            detail: Some("Required for building from source. Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh".into()),
            tier_impact: Some("Prebuilt binary can be used instead".into()),
        });
    }

    // Build tools (cc, pkg-config)
    let cc_ok = Command::new("cc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let pkg_ok = Command::new("pkg-config")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if cc_ok && pkg_ok {
        checks.push(CheckResult {
            category: "Dependencies",
            label: "Build tools: cc, pkg-config present".into(),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else {
        let mut missing = Vec::new();
        if !cc_ok {
            missing.push("build-essential");
        }
        if !pkg_ok {
            missing.push("pkg-config");
        }
        checks.push(CheckResult {
            category: "Dependencies",
            label: format!("Build tools: missing {}", missing.join(", ")),
            status: CheckStatus::Warn,
            detail: Some(format!(
                "Install: sudo apt install -y {}",
                missing.join(" ")
            )),
            tier_impact: Some("Required for building from source".into()),
        });
    }

    // Git
    if Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        checks.push(CheckResult {
            category: "Dependencies",
            label: "Git: installed".into(),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else {
        checks.push(CheckResult {
            category: "Dependencies",
            label: "Git: not installed".into(),
            status: CheckStatus::Warn,
            detail: Some("Install: sudo apt install -y git".into()),
            tier_impact: None,
        });
    }

    // curl
    if Command::new("curl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        checks.push(CheckResult {
            category: "Dependencies",
            label: "curl: installed".into(),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    }

    // ═══════════════════════════════════════════
    // CATEGORY: Ollama / Semantic Stage
    // ═══════════════════════════════════════════

    let config = crate::config::load_config(Some(config_path));
    let semantic_model = &config.semantic_model;
    let semantic_endpoint = &config.semantic_endpoint;

    if let Some(ollama_ver) = check_ollama_installed() {
        checks.push(CheckResult {
            category: "Semantic",
            label: format!("Ollama: {}", ollama_ver),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });

        // Ollama service
        if check_ollama_running() {
            checks.push(CheckResult {
                category: "Semantic",
                label: "Ollama service: running (port 11434)".into(),
                status: CheckStatus::Pass,
                detail: None,
                tier_impact: None,
            });

            // API health
            if check_ollama_health(semantic_endpoint) {
                checks.push(CheckResult {
                    category: "Semantic",
                    label: "Ollama API: responding".into(),
                    status: CheckStatus::Pass,
                    detail: None,
                    tier_impact: None,
                });
            } else {
                checks.push(CheckResult {
                    category: "Semantic",
                    label: "Ollama API: not responding".into(),
                    status: CheckStatus::Warn,
                    detail: Some(format!(
                        "Endpoint {} unreachable. Check OLLAMA_HOST config.",
                        semantic_endpoint
                    )),
                    tier_impact: None,
                });
            }
        } else {
            checks.push(CheckResult {
                category: "Semantic",
                label: "Ollama service: not running".into(),
                status: CheckStatus::Warn,
                detail: Some("Start with: sudo systemctl start ollama".into()),
                tier_impact: Some("Semantic stage unavailable until started".into()),
            });
        }

        // Model pulled
        let model_short = semantic_model.split(':').next().unwrap_or(semantic_model);
        if check_ollama_model(model_short) {
            checks.push(CheckResult {
                category: "Semantic",
                label: format!("Model: {} — pulled", semantic_model),
                status: CheckStatus::Pass,
                detail: None,
                tier_impact: None,
            });
        } else {
            checks.push(CheckResult {
                category: "Semantic",
                label: format!("Model: {} — not pulled", semantic_model),
                status: CheckStatus::Warn,
                detail: Some(format!("Pull with: ollama pull {}", semantic_model)),
                tier_impact: Some("Semantic stage won't function until model is available".into()),
            });
        }

        // OLLAMA_KEEP_ALIVE check
        #[cfg(target_os = "linux")]
        {
            let keep_alive =
                std::fs::read_to_string("/etc/systemd/system/ollama.service.d/nyquest.conf")
                    .ok()
                    .map(|s| s.contains("OLLAMA_KEEP_ALIVE=-1"))
                    .unwrap_or(false);
            if keep_alive {
                checks.push(CheckResult {
                    category: "Semantic",
                    label: "OLLAMA_KEEP_ALIVE=-1 configured (persistent VRAM)".into(),
                    status: CheckStatus::Pass,
                    detail: None,
                    tier_impact: None,
                });
            } else {
                checks.push(CheckResult {
                    category: "Semantic",
                    label: "OLLAMA_KEEP_ALIVE not configured".into(),
                    status: CheckStatus::Warn,
                    detail: Some("Without KEEP_ALIVE=-1, model unloads from VRAM after idle. First call after idle will be slow (~3-5s).".into()),
                    tier_impact: Some("Higher semantic latency on cold starts".into()),
                });
            }
        }
    } else {
        checks.push(CheckResult {
            category: "Semantic",
            label: "Ollama: not installed".into(),
            status: if config.semantic_enabled {
                CheckStatus::Fail
            } else {
                CheckStatus::Info
            },
            detail: Some("Install: curl -fsSL https://ollama.com/install.sh | sh".into()),
            tier_impact: if config.semantic_enabled {
                Some("Semantic compression requires Ollama".into())
            } else {
                Some("Optional — only needed for Tier 2/3 semantic compression".into())
            },
        });
    }

    // Semantic config status
    checks.push(CheckResult {
        category: "Semantic",
        label: format!(
            "semantic_enabled: {}",
            if config.semantic_enabled {
                "true"
            } else {
                "false"
            }
        ),
        status: CheckStatus::Info,
        detail: if !config.semantic_enabled {
            Some("Enable in nyquest.yaml or: nyquest config set semantic_enabled true".into())
        } else {
            None
        },
        tier_impact: None,
    });

    // ═══════════════════════════════════════════
    // CATEGORY: Network / Ports
    // ═══════════════════════════════════════════

    let nyquest_port = config.port;

    if check_port_responding(nyquest_port) {
        checks.push(CheckResult {
            category: "Network",
            label: format!("Port {} — Nyquest responding", nyquest_port),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else if check_port_available(nyquest_port) {
        checks.push(CheckResult {
            category: "Network",
            label: format!("Port {} — available (Nyquest not running)", nyquest_port),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else {
        checks.push(CheckResult {
            category: "Network",
            label: format!("Port {} — in use by another process", nyquest_port),
            status: CheckStatus::Fail,
            detail: Some(format!(
                "Free the port or change in nyquest.yaml: port: {}",
                nyquest_port + 1
            )),
            tier_impact: Some("Cannot start Nyquest".into()),
        });
    }

    // Ollama port
    if check_port_responding(11434) {
        checks.push(CheckResult {
            category: "Network",
            label: "Port 11434 — Ollama responding".into(),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else {
        checks.push(CheckResult {
            category: "Network",
            label: "Port 11434 — Ollama not responding".into(),
            status: if config.semantic_enabled {
                CheckStatus::Warn
            } else {
                CheckStatus::Info
            },
            detail: if config.semantic_enabled {
                Some("Semantic stage requires Ollama on port 11434".into())
            } else {
                Some("Not needed unless semantic_enabled: true".into())
            },
            tier_impact: None,
        });
    }

    // ═══════════════════════════════════════════
    // CATEGORY: Nyquest Binary
    // ═══════════════════════════════════════════

    let exe = std::env::current_exe().unwrap_or_default();
    checks.push(CheckResult {
        category: "Binary",
        label: format!("Nyquest v{}", crate::VERSION),
        status: CheckStatus::Pass,
        detail: Some(format!("Binary: {}", exe.display())),
        tier_impact: None,
    });

    // Binary size
    if let Ok(meta) = std::fs::metadata(&exe) {
        let size_mb = meta.len() as f64 / 1024.0 / 1024.0;
        checks.push(CheckResult {
            category: "Binary",
            label: format!("Binary size: {:.1} MB", size_mb),
            status: CheckStatus::Info,
            detail: None,
            tier_impact: None,
        });
    }

    // Config file
    if Path::new(config_path).exists() {
        checks.push(CheckResult {
            category: "Binary",
            label: format!("Config: {} — found", config_path),
            status: CheckStatus::Pass,
            detail: None,
            tier_impact: None,
        });
    } else {
        checks.push(CheckResult {
            category: "Binary",
            label: format!("Config: {} — not found", config_path),
            status: CheckStatus::Warn,
            detail: Some("Run: nyquest install  to generate config".into()),
            tier_impact: None,
        });
    }

    // ═══════════════════════════════════════════
    // RENDER OUTPUT
    // ═══════════════════════════════════════════

    println!();
    println!(
        "  {}",
        style("╔══════════════════════════════════════════════════════════╗").cyan()
    );
    println!(
        "  {}  {}  {}",
        style("║").cyan(),
        style(format!(
            "NYQUEST v{} — SYSTEM PREFLIGHT CHECK",
            crate::VERSION
        ))
        .cyan()
        .bold(),
        style("     ║").cyan(),
    );
    println!(
        "  {}",
        style("╚══════════════════════════════════════════════════════════╝").cyan()
    );
    println!();

    let mut current_category = "";
    let mut pass_count = 0u32;
    let mut warn_count = 0u32;
    let mut fail_count = 0u32;
    let mut info_count = 0u32;

    for check in &checks {
        if check.category != current_category {
            if !current_category.is_empty() {
                println!();
            }
            println!(
                "  {}  {}",
                style("▸").cyan(),
                style(check.category).cyan().bold()
            );
            println!("  {}", style("─".repeat(56)).dim());
            current_category = check.category;
        }

        let icon = match check.status {
            CheckStatus::Pass => {
                pass_count += 1;
                style("  ✓").green()
            }
            CheckStatus::Warn => {
                warn_count += 1;
                style("  ⚠").yellow()
            }
            CheckStatus::Fail => {
                fail_count += 1;
                style("  ✗").red()
            }
            CheckStatus::Info => {
                info_count += 1;
                style("  ℹ").dim()
            }
        };

        println!("{} {}", icon, check.label);

        if verbose {
            if let Some(ref detail) = check.detail {
                println!("      {}", style(detail).dim());
            }
            if let Some(ref tier) = check.tier_impact {
                println!("      {}", style(format!("→ {}", tier)).yellow());
            }
        } else if check.status == CheckStatus::Fail || check.status == CheckStatus::Warn {
            if let Some(ref detail) = check.detail {
                println!("      {}", style(detail).dim());
            }
        }
    }

    // ═══════════════════════════════════════════
    // TIER RECOMMENDATION
    // ═══════════════════════════════════════════

    println!();
    println!("  {}", style("═".repeat(56)).cyan());
    println!();

    let total = pass_count + warn_count + fail_count;
    println!(
        "  {} passed  {} warnings  {} errors  {} info",
        style(pass_count).green().bold(),
        style(warn_count).yellow().bold(),
        style(fail_count).red().bold(),
        style(info_count).dim(),
    );

    println!();
    match recommended_tier {
        Tier::Tier2GpuSemantic => {
            println!(
                "  {} {}",
                style("⚡ RECOMMENDED TIER:").cyan().bold(),
                style("Tier 2 — GPU Semantic (full pipeline)")
                    .green()
                    .bold(),
            );
            println!("      Rule compression (350+ rules, <2ms) + GPU-accelerated semantic");
            println!("      condensation (Qwen 2.5 1.5B, 200-350ms). Full 15-75% savings.");
        }
        Tier::Tier3CpuSemantic => {
            println!(
                "  {} {}",
                style("⚡ RECOMMENDED TIER:").cyan().bold(),
                style("Tier 3 — CPU Semantic").yellow().bold(),
            );
            println!("      Rule compression (350+ rules, <2ms) + CPU-based semantic");
            println!("      condensation (Qwen 2.5 1.5B via Ollama CPU, 1-4s latency).");
            println!("      Add a GPU (2+ GB VRAM) for Tier 2 performance.");
        }
        Tier::Tier1RulesOnly => {
            println!(
                "  {} {}",
                style("⚡ RECOMMENDED TIER:").cyan().bold(),
                style("Tier 1 — Rules Only").white().bold(),
            );
            println!("      Rule compression only (350+ rules, <2ms, 15-37% savings).");
            println!("      Add 6+ GB RAM + Ollama for semantic stage (56-75% savings).");
        }
        Tier::Insufficient => {
            println!(
                "  {} {}",
                style("⚡ SYSTEM STATUS:").red().bold(),
                style("Below minimum requirements").red().bold(),
            );
            println!("      Resolve the errors above before proceeding.");
            println!("      Minimum: 2 cores, 512 MB RAM, glibc 2.31+, 50 MB disk.");
        }
    }

    // Quick fix suggestions for failures
    let failures: Vec<&CheckResult> = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
        .collect();

    if !failures.is_empty() {
        println!();
        println!("  {}", style("FIX REQUIRED:").red().bold());
        for f in failures {
            println!("    {} {}", style("•").red(), f.label);
            if let Some(ref d) = f.detail {
                println!("      {}", style(d).dim());
            }
        }
    }

    // Semantic setup suggestion
    if recommended_tier == Tier::Tier2GpuSemantic || recommended_tier == Tier::Tier3CpuSemantic {
        let ollama_installed = check_ollama_installed().is_some();
        if !ollama_installed || !config.semantic_enabled {
            println!();
            println!(
                "  {}",
                style("NEXT STEPS for semantic compression:").cyan().bold()
            );
            if !ollama_installed {
                println!(
                    "    1. Install Ollama:     {}",
                    style("curl -fsSL https://ollama.com/install.sh | sh").green()
                );
                println!(
                    "    2. Pull model:         {}",
                    style(format!("ollama pull {}", semantic_model)).green()
                );
            } else if !check_ollama_model(
                semantic_model.split(':').next().unwrap_or(semantic_model),
            ) {
                println!(
                    "    1. Pull model:         {}",
                    style(format!("ollama pull {}", semantic_model)).green()
                );
            }
            if !config.semantic_enabled {
                println!(
                    "    {}. Enable semantic:    {}",
                    if !ollama_installed { "3" } else { "2" },
                    style("nyquest config set semantic_enabled true").green()
                );
            }
            println!(
                "    Or run the full setup: {}",
                style("docs/semantic-stage/setup_semantic.sh").green()
            );
        }
    }

    println!();
}
