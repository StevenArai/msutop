//! macOS-only read-only sensor helper. Not part of the root Cargo project.
#[cfg(target_os = "macos")]
mod macos {
    use macmon::Sampler;
    use serde_json::json;
    use std::io::{self, Write};
    use std::time::{Duration, SystemTime};

    unsafe extern "C" {
        fn getppid() -> i32;
    }
    fn parent() -> i32 {
        unsafe { getppid() }
    }
    fn emit(value: serde_json::Value) {
        println!("{value}");
        let _ = io::stdout().flush();
    }
    fn finite(value: f64, min: f64, max: f64) -> Option<f64> {
        (value.is_finite() && value >= min && value <= max).then_some(value)
    }

    fn run(samples: usize) -> Result<(), Box<dyn std::error::Error>> {
        let owner = parent();
        if owner <= 1 {
            return Ok(());
        }
        // Strict CLPC prevents macOS 27's frozen legacy energy counters being presented as 0 W.
        let mut sampler = Sampler::with_clpc()?;
        if sampler.get_soc_info().vm {
            return Err("Hardware sensors are unavailable in this VM".into());
        }
        let mut count = 0;
        loop {
            if parent() != owner {
                return Ok(());
            }
            let started = SystemTime::now();
            let metrics = sampler.get_metrics(500)?;
            if started.elapsed().unwrap_or_default() > Duration::from_secs(5) {
                // Discard a delta that spans sleep and establish a fresh hardware baseline.
                sampler = Sampler::with_clpc()?;
                continue;
            }
            let tier = metrics.cpu_tiers.last();
            let frequency = tier.and_then(|t| finite(t.freq_mhz as f64, 100.0, 10_000.0));
            emit(json!({
                "type": "sample",
                "cpu_power_watts": finite(metrics.cpu_power as f64, 0.0, 1000.0),
                "cpu_temperature_c": metrics.temp.cpu_temp_avg.and_then(|v| finite(v as f64, 0.1, 150.0)),
                "cpu_frequency_mhz": frequency,
                "frequency_tier": tier.map(|t| t.label.as_str()),
                "source": "macmon-clpc-smc"
            }));
            count += 1;
            if samples > 0 && count >= samples {
                return Ok(());
            }
        }
    }
    pub fn main() {
        let args: Vec<_> = std::env::args().collect();
        let samples = args
            .iter()
            .position(|a| a == "--samples")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if let Err(error) = run(samples) {
            emit(json!({"type": "error", "message": error.to_string()}));
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("msutop-sensors is a macOS-only helper");
    std::process::exit(1);
}
