use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealModelBackend {
    TensorRtEngine,
    TensorRtOnnx,
    SmolVlmCuda,
}

#[derive(Clone, Debug)]
pub struct RealModelConfig {
    pub backend: RealModelBackend,
    pub model: String,
    pub image_path: Option<PathBuf>,
    pub prompt: Option<String>,
    pub max_new_tokens: u32,
    pub extra_args: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RealModelRun {
    pub ok: bool,
    pub backend: RealModelBackend,
    pub model: String,
    pub latency_ms: u64,
    pub output_tokens: Option<u32>,
    pub peak_cuda_allocated_mb: Option<f32>,
    pub exit_code: Option<i32>,
    pub stdout_digest: String,
    pub stderr_digest: String,
}

impl RealModelBackend {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tensorrt-engine" | "trt-engine" | "engine" => Some(Self::TensorRtEngine),
            "tensorrt-onnx" | "trt-onnx" | "onnx" => Some(Self::TensorRtOnnx),
            "smolvlm-cuda" | "smolvlm" => Some(Self::SmolVlmCuda),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::TensorRtEngine => "tensorrt-engine",
            Self::TensorRtOnnx => "tensorrt-onnx",
            Self::SmolVlmCuda => "smolvlm-cuda",
        }
    }
}

pub fn run_real_model(config: &RealModelConfig) -> io::Result<RealModelRun> {
    let started = Instant::now();
    let output = build_command(config).output()?;
    let latency_ms = started.elapsed().as_millis().max(1) as u64;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Ok(RealModelRun {
        ok: output.status.success(),
        backend: config.backend,
        model: config.model.clone(),
        latency_ms,
        output_tokens: parse_metric_u32(&stdout, &["tokens:", "tokens="]),
        peak_cuda_allocated_mb: parse_peak_cuda_mb(&stdout),
        exit_code: output.status.code(),
        stdout_digest: compact_output(&stdout),
        stderr_digest: compact_output(&stderr),
    })
}

pub fn build_command(config: &RealModelConfig) -> Command {
    match config.backend {
        RealModelBackend::TensorRtEngine => {
            let mut command = Command::new(trtexec_binary());
            command
                .arg(format!("--loadEngine={}", config.model))
                .arg("--iterations=1")
                .arg("--duration=0");
            append_extra_args(&mut command, &config.extra_args);
            command
        }
        RealModelBackend::TensorRtOnnx => {
            let mut command = Command::new(trtexec_binary());
            command
                .arg(format!("--onnx={}", config.model))
                .arg("--iterations=1")
                .arg("--duration=0");
            append_extra_args(&mut command, &config.extra_args);
            command
        }
        RealModelBackend::SmolVlmCuda => {
            let mut command = Command::new("python3");
            command
                .arg("scripts/talos_smolvlm_probe.py")
                .arg("--model-id")
                .arg(&config.model)
                .arg("--max-new-tokens")
                .arg(config.max_new_tokens.to_string());
            if let Some(image_path) = &config.image_path {
                command.arg("--image-path").arg(image_path);
            }
            if let Some(prompt) = &config.prompt {
                command.arg("--prompt").arg(prompt);
            }
            append_extra_args(&mut command, &config.extra_args);
            command
        }
    }
}

fn trtexec_binary() -> String {
    std::env::var("TALOS_TRTEXEC").unwrap_or_else(|_| "trtexec".to_string())
}

fn append_extra_args(command: &mut Command, extra_args: &[String]) {
    for arg in extra_args {
        command.arg(arg);
    }
}

fn parse_metric_u32(output: &str, labels: &[&str]) -> Option<u32> {
    for line in output.lines() {
        let trimmed = line.trim();
        for label in labels {
            if let Some(value) = trimmed.strip_prefix(label) {
                return value.trim().parse().ok();
            }
        }
    }
    None
}

fn parse_peak_cuda_mb(output: &str) -> Option<f32> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("peak_cuda_allocated_gb:") {
            let gb: f32 = value.trim().parse().ok()?;
            return Some(gb * 1024.0);
        }
        if let Some(value) = trimmed.strip_prefix("peak_cuda_allocated_mb:") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn compact_output(output: &str) -> String {
    let mut compact = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .join(" | ");
    if compact.len() > 512 {
        compact.truncate(512);
    }
    compact
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_backend_names() {
        assert_eq!(
            RealModelBackend::parse("tensorrt-engine"),
            Some(RealModelBackend::TensorRtEngine)
        );
        assert_eq!(
            RealModelBackend::parse("onnx"),
            Some(RealModelBackend::TensorRtOnnx)
        );
        assert_eq!(
            RealModelBackend::parse("smolvlm"),
            Some(RealModelBackend::SmolVlmCuda)
        );
    }

    #[test]
    fn parses_smolvlm_metrics() {
        let stdout = "tokens: 24\npeak_cuda_allocated_gb: 2.50\n";
        assert_eq!(parse_metric_u32(stdout, &["tokens:"]), Some(24));
        assert_eq!(parse_peak_cuda_mb(stdout), Some(2560.0));
    }

    #[test]
    fn builds_trtexec_engine_command() {
        let command = build_command(&RealModelConfig {
            backend: RealModelBackend::TensorRtEngine,
            model: "/models/yolo.engine".to_string(),
            image_path: None,
            prompt: None,
            max_new_tokens: 0,
            extra_args: vec!["--verbose".to_string()],
        });

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(args.contains(&"--loadEngine=/models/yolo.engine".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn trtexec_path_can_be_overridden() {
        std::env::set_var("TALOS_TRTEXEC", "/opt/tensorrt/bin/trtexec");
        let command = build_command(&RealModelConfig {
            backend: RealModelBackend::TensorRtOnnx,
            model: "/models/vision.onnx".to_string(),
            image_path: None,
            prompt: None,
            max_new_tokens: 0,
            extra_args: Vec::new(),
        });

        assert_eq!(
            command.get_program().to_string_lossy(),
            "/opt/tensorrt/bin/trtexec"
        );
        std::env::remove_var("TALOS_TRTEXEC");
    }
}
