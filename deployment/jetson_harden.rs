use talos::{JetsonHardeningConfig, JetsonHardeningPlan};

#[derive(Clone, Debug)]
struct Args {
    apply: bool,
    status: bool,
    restore_clocks: bool,
    config: JetsonHardeningConfig,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let plan = if args.restore_clocks {
        JetsonHardeningConfig::restore_plan()
    } else if args.status {
        JetsonHardeningPlan::status_plan()
    } else {
        args.config.plan()
    };

    println!("{}", plan.render());

    if args.status || args.apply {
        let outcomes = plan.execute();
        let failed = outcomes.iter().filter(|outcome| !outcome.success).count();

        for outcome in outcomes {
            println!(
                "outcome label={} status={:?} success={}",
                outcome.label, outcome.status_code, outcome.success
            );
        }

        if failed > 0 {
            return Err(format!("{failed} deployment command(s) failed").into());
        }
    } else {
        println!("dry_run=true");
        println!("pass --apply to execute mutating commands");
    }

    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut apply = false;
    let mut status = false;
    let mut restore_clocks = false;
    let mut config = JetsonHardeningConfig::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => apply = true,
            "--status" => status = true,
            "--restore-clocks" => restore_clocks = true,
            "--no-clocks" => config.lock_clocks = false,
            "--no-nvpmodel" => config.nvpmodel_mode = None,
            "--mode" => {
                let value = args.next().ok_or("--mode requires a value")?;
                config.nvpmodel_mode = Some(value.parse()?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    if restore_clocks && status {
        return Err("--restore-clocks cannot be combined with --status".into());
    }

    Ok(Args {
        apply,
        status,
        restore_clocks,
        config,
    })
}

fn print_help() {
    println!(
        "Usage: jetson_harden [--status] [--apply] [--mode N] [--no-nvpmodel] [--no-clocks] [--restore-clocks]"
    );
    println!();
    println!("Default is dry-run for Orin Nano style hardening: nvpmodel mode 0 + jetson_clocks.");
    println!("Use --status to execute non-mutating probes.");
    println!("Use --apply to execute mutating hardening commands.");
}
