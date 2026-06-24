use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JetsonHardeningConfig {
    pub nvpmodel_mode: Option<u8>,
    pub lock_clocks: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentCommand {
    pub label: &'static str,
    pub command: String,
    pub mutates_system: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JetsonHardeningPlan {
    pub commands: Vec<DeploymentCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutcome {
    pub label: &'static str,
    pub command: String,
    pub status_code: Option<i32>,
    pub success: bool,
}

impl Default for JetsonHardeningConfig {
    fn default() -> Self {
        Self {
            nvpmodel_mode: Some(0),
            lock_clocks: true,
        }
    }
}

impl JetsonHardeningConfig {
    pub fn plan(&self) -> JetsonHardeningPlan {
        let mut commands = Vec::new();

        if let Some(mode) = self.nvpmodel_mode {
            commands.push(DeploymentCommand {
                label: "set_nvpmodel",
                command: format!("sudo nvpmodel -m {mode}"),
                mutates_system: true,
            });
        }

        if self.lock_clocks {
            commands.push(DeploymentCommand {
                label: "lock_clocks",
                command: "sudo jetson_clocks".to_string(),
                mutates_system: true,
            });
        }

        commands.extend(status_commands());

        JetsonHardeningPlan { commands }
    }

    pub fn restore_plan() -> JetsonHardeningPlan {
        JetsonHardeningPlan {
            commands: vec![
                DeploymentCommand {
                    label: "restore_clocks",
                    command: "sudo jetson_clocks --restore".to_string(),
                    mutates_system: true,
                },
                DeploymentCommand {
                    label: "show_clocks",
                    command: "sudo -n jetson_clocks --show 2>/dev/null || true".to_string(),
                    mutates_system: false,
                },
            ],
        }
    }
}

impl JetsonHardeningPlan {
    pub fn status_plan() -> Self {
        Self {
            commands: status_commands(),
        }
    }

    pub fn render(&self) -> String {
        self.commands
            .iter()
            .map(|command| {
                let kind = if command.mutates_system {
                    "apply"
                } else {
                    "status"
                };
                format!("[{kind}] {}: {}", command.label, command.command)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn execute(&self) -> Vec<CommandOutcome> {
        self.commands
            .iter()
            .map(|command| {
                let status = Command::new("sh").args(["-lc", &command.command]).status();
                match status {
                    Ok(status) => CommandOutcome {
                        label: command.label,
                        command: command.command.clone(),
                        status_code: status.code(),
                        success: status.success(),
                    },
                    Err(_) => CommandOutcome {
                        label: command.label,
                        command: command.command.clone(),
                        status_code: None,
                        success: false,
                    },
                }
            })
            .collect()
    }
}

fn status_commands() -> Vec<DeploymentCommand> {
    vec![
        DeploymentCommand {
            label: "l4t_release",
            command: "cat /etc/nv_tegra_release 2>/dev/null || true".to_string(),
            mutates_system: false,
        },
        DeploymentCommand {
            label: "architecture",
            command: "uname -m".to_string(),
            mutates_system: false,
        },
        DeploymentCommand {
            label: "nvpmodel_status",
            command: "nvpmodel -q 2>/dev/null || true".to_string(),
            mutates_system: false,
        },
        DeploymentCommand {
            label: "clock_status",
            command: "sudo -n jetson_clocks --show 2>/dev/null || true".to_string(),
            mutates_system: false,
        },
        DeploymentCommand {
            label: "tegrastats_sample",
            command:
                "tegrastats --interval 1000 2>/dev/null & pid=$!; sleep 2; kill $pid 2>/dev/null || true; wait $pid 2>/dev/null || true"
                    .to_string(),
            mutates_system: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_plan_sets_power_mode_and_locks_clocks() {
        let plan = JetsonHardeningConfig::default().plan();

        assert!(plan
            .commands
            .iter()
            .any(|command| command.command == "sudo nvpmodel -m 0"));
        assert!(plan
            .commands
            .iter()
            .any(|command| command.command == "sudo jetson_clocks"));
        assert!(plan.commands.iter().any(|command| !command.mutates_system));
    }

    #[test]
    fn no_clock_plan_only_sets_power_mode_and_status() {
        let plan = JetsonHardeningConfig {
            nvpmodel_mode: Some(1),
            lock_clocks: false,
        }
        .plan();

        assert!(plan
            .commands
            .iter()
            .any(|command| command.command == "sudo nvpmodel -m 1"));
        assert!(!plan
            .commands
            .iter()
            .any(|command| command.command == "sudo jetson_clocks"));
    }

    #[test]
    fn status_plan_does_not_mutate_system() {
        let plan = JetsonHardeningPlan::status_plan();

        assert!(!plan.commands.is_empty());
        assert!(plan.commands.iter().all(|command| !command.mutates_system));
    }

    #[test]
    fn rendered_plan_marks_mutating_commands() {
        let rendered = JetsonHardeningConfig::default().plan().render();

        assert!(rendered.contains("[apply] set_nvpmodel"));
        assert!(rendered.contains("[status] nvpmodel_status"));
    }
}
