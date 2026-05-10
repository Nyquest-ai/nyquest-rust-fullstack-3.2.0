//! Nyquest Doctor — Health check & connectivity validation

use crate::config::load_config;
use console::style;
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

struct Check {
    label: String,
    status: Status,
}

enum Status {
    Pass,
    Warn,
    Fail,
}

pub fn run_doctor(config_path: &str) {
    let mut checks: Vec<Check> = Vec::new();

    // Rust version
    checks.push(Check {
        label: format!("Nyquest v{} (Rust engine)", crate::VERSION),
        status: Status::Pass,
    });

    // Config file
    let path = Path::new(config_path);
    if path.exists() {
        let config = load_config(Some(config_path));
        checks.push(Check {
            label: format!("Config found: {}", config_path),
            status: Status::Pass,
        });

        // Port check
        let addr = format!("127.0.0.1:{}", config.port);
        match TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_secs(2)) {
            Ok(_) => checks.push(Check {
                label: format!("Port {} — Nyquest responding", config.port),
                status: Status::Pass,
            }),
            Err(_) => checks.push(Check {
                label: format!("Port {} — Nyquest not running", config.port),
                status: Status::Warn,
            }),
        }

        // Provider keys
        for (name, pconf) in &config.providers {
            if let Some(key) = pconf.get("api_key") {
                if key.len() > 10 {
                    checks.push(Check {
                        label: format!("{} API key configured", name),
                        status: Status::Pass,
                    });
                } else {
                    checks.push(Check {
                        label: format!("{} API key too short", name),
                        status: Status::Fail,
                    });
                }
            } else {
                checks.push(Check {
                    label: format!("{} API key not configured", name),
                    status: Status::Warn,
                });
            }
        }

        // Dashboard
        match TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_secs(2)) {
            Ok(_) => checks.push(Check {
                label: format!(
                    "Dashboard accessible at http://localhost:{}/dashboard/",
                    config.port
                ),
                status: Status::Pass,
            }),
            Err(_) => checks.push(Check {
                label: "Dashboard not accessible".into(),
                status: Status::Warn,
            }),
        }

        // Log dir
        if Path::new(&config.log_file)
            .parent()
            .map(|p| p.exists())
            .unwrap_or(false)
        {
            checks.push(Check {
                label: "Log directory writable".into(),
                status: Status::Pass,
            });
        } else {
            checks.push(Check {
                label: "Log directory missing".into(),
                status: Status::Warn,
            });
        }
    } else {
        checks.push(Check {
            label: format!("Config not found: {}", config_path),
            status: Status::Fail,
        });
    }

    // Systemd service
    if let Ok(output) = Command::new("systemctl")
        .args(["--user", "is-active", "nyquest.service"])
        .output()
    {
        let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if status_str == "active" {
            checks.push(Check {
                label: "Systemd service active".into(),
                status: Status::Pass,
            });
        } else {
            checks.push(Check {
                label: format!("Systemd service: {}", status_str),
                status: Status::Warn,
            });
        }
    }

    // Print results
    println!();
    let mut passed = 0u32;
    let mut warned = 0u32;
    let mut failed = 0u32;

    for c in &checks {
        match c.status {
            Status::Pass => {
                println!("  {} {}", style("✓").green(), c.label);
                passed += 1;
            }
            Status::Warn => {
                println!("  {} {}", style("⚠").yellow(), c.label);
                warned += 1;
            }
            Status::Fail => {
                println!("  {} {}", style("✗").red(), c.label);
                failed += 1;
            }
        }
    }

    let total = checks.len();
    println!();
    println!(
        "  Result: {}/{} passed, {} warnings, {} errors",
        passed, total, warned, failed
    );
    println!();
}

pub fn run_status(config_path: &str) {
    let path = Path::new(config_path);
    println!();
    println!("  {}", style("Nyquest Status").magenta().bold());
    println!("  {}", style("─".repeat(30)).dim());

    if path.exists() {
        let config = load_config(Some(config_path));
        println!("  {} Config: {}", style("✓").green(), config_path);
        println!("    Compression level: {}", config.compression_level);
        println!("    OpenClaw mode: {}", config.openclaw_mode);
        println!("    Port: {}", config.port);

        // Service
        if let Ok(output) = Command::new("systemctl")
            .args(["--user", "is-active", "nyquest.service"])
            .output()
        {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if s == "active" {
                println!("  {} Service: {}", style("✓").green(), s);
            } else {
                println!("  {} Service: {}", style("⚠").yellow(), s);
            }
        }

        // Port
        let addr = format!("127.0.0.1:{}", config.port);
        match TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_secs(1)) {
            Ok(_) => println!(
                "  {} Proxy responding on port {}",
                style("✓").green(),
                config.port
            ),
            Err(_) => println!(
                "  {} Proxy not responding on port {}",
                style("⚠").yellow(),
                config.port
            ),
        }
    } else {
        println!("  {} No config at {}", style("⚠").yellow(), config_path);
    }
    println!();
}
