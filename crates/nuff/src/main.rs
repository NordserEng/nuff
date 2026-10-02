use std::io::Write;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use colored::Colorize;

use nuff::args::Args;
use nuff::{ExitStatus, run};

// The Linux wheels cross-compile through zig, which cannot link jemalloc, so Linux keeps the system
// allocator.
#[cfg(target_os = "macos")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> ExitCode {
    let args = wild::args_os();
    let args = match ruff_command_line::expand_args(args)
        .context("Failed to read CLI arguments from files")
    {
        Ok(args) => args,
        Err(err) => return report_error(&err),
    };

    let args = Args::parse_from(args);

    match run(args) {
        Ok(code) => code.into(),
        Err(err) => report_error(&err),
    }
}

fn report_error(err: &anyhow::Error) -> ExitCode {
    for cause in err.chain() {
        if let Some(ioerr) = cause.downcast_ref::<std::io::Error>() {
            if ioerr.kind() == std::io::ErrorKind::BrokenPipe {
                return ExitCode::from(0);
            }
        }
    }

    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "{}", "nuff failed".red().bold()).ok();
    for cause in err.chain() {
        writeln!(stderr, "  {} {cause}", "Cause:".bold()).ok();
    }
    ExitStatus::Error.into()
}
