#![allow(clippy::print_stdout)]

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use log::error;

use args::GlobalConfigArgs;
use nuff_db::diagnostic::{Diagnostic, Severity};
use nuff_linter::logging::set_up_logging;
use nuff_linter::settings::flags::FixMode;
use nuff_linter::{fs, warn_user_once};
use nuff_workspace::Settings;

use crate::args::{Args, CheckCommand, Command, TerminalColor};
use crate::printer::{Flags as PrinterFlags, Printer};

pub mod args;
mod cache;
mod commands;
mod diagnostics;
mod printer;
pub mod resolve;
mod stdin;

#[derive(Copy, Clone)]
pub enum ExitStatus {
    /// Linting was successful and there were no linting errors.
    Success,
    /// Linting was successful but there were linting errors.
    Failure,
    /// Linting failed.
    Error,
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> Self {
        match status {
            ExitStatus::Success => ExitCode::from(0),
            ExitStatus::Failure => ExitCode::from(1),
            ExitStatus::Error => ExitCode::from(2),
        }
    }
}

/// Returns true if the command should read from standard input.
fn is_stdin(files: &[PathBuf], stdin_filename: Option<&Path>) -> bool {
    // If the user provided a `--stdin-filename`, always read from standard input.
    if stdin_filename.is_some() {
        if let Some(file) = files.iter().find(|file| file.as_path() != Path::new("-")) {
            warn_user_once!(
                "Ignoring file {} in favor of standard input.",
                file.display()
            );
        }
        return true;
    }

    let [file] = files else {
        return false;
    };
    // If the user provided exactly `-`, read from standard input.
    file == Path::new("-")
}

/// Returns the default set of files if none are provided, otherwise returns provided files.
fn resolve_default_files(files: Vec<PathBuf>, is_stdin: bool) -> Vec<PathBuf> {
    if files.is_empty() {
        if is_stdin {
            vec![Path::new("-").to_path_buf()]
        } else {
            vec![Path::new(".").to_path_buf()]
        }
    } else {
        files
    }
}

pub fn run(
    Args {
        command,
        global_options,
    }: Args,
) -> Result<ExitStatus> {
    if let Some(color_override) =
        colored_override(global_options.color, std::env::var_os("FORCE_COLOR"))
    {
        colored::control::set_override(color_override);
    }
    set_up_logging(global_options.log_level())?;

    match command {
        Command::Check(args) => check(args, global_options),
    }
}

pub fn check(args: CheckCommand, global_options: GlobalConfigArgs) -> Result<ExitStatus> {
    let (cli, config_arguments) = args.partition(global_options)?;

    // Construct the "default" settings. These are used when no `pyproject.toml`
    // files are present, or files are injected from outside of the hierarchy.
    let pyproject_config = resolve::resolve(&config_arguments, cli.stdin_filename.as_deref())?;

    let writer: Box<dyn Write> = match cli.output_file {
        Some(path) => {
            colored::control::set_override(false);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = File::create(path)?;
            Box::new(BufWriter::new(file))
        }
        _ => Box::new(BufWriter::new(io::stdout())),
    };
    let stderr_writer = Box::new(BufWriter::new(io::stderr()));

    let is_stdin = is_stdin(&cli.files, cli.stdin_filename.as_deref());
    let files = resolve_default_files(cli.files, is_stdin);

    // Extract options that are included in `Settings`, but only apply at the top
    // level.
    let Settings {
        fix,
        fix_only,
        unsafe_fixes,
        output_format,
        show_fixes,
        ..
    } = pyproject_config.settings;

    // Fix rules are as follows:
    // - By default, generate all fixes, but don't apply them to the filesystem.
    // - If `--fix` or `--fix-only` is set, apply applicable fixes to the filesystem (or
    //   print them to stdout, if we're reading from stdin).
    // - If `--diff` or `--fix-only` are set, don't print any violations (only applicable fixes)
    // - By default, applicable fixes only include [`Applicability::Automatic`], but if
    //   `--unsafe-fixes` is set, then [`Applicability::Suggested`] fixes are included.

    let fix_mode = if cli.diff {
        FixMode::Diff
    } else if fix || fix_only {
        FixMode::Apply
    } else {
        FixMode::Generate
    };

    let cache = !cli.no_cache;
    let noqa = !cli.ignore_noqa;
    let mut printer_flags = PrinterFlags::empty();
    if !(cli.diff || fix_only) {
        printer_flags |= PrinterFlags::SHOW_VIOLATIONS;
    }
    if show_fixes {
        printer_flags |= PrinterFlags::SHOW_FIX_SUMMARY;
    }

    #[cfg(debug_assertions)]
    if cache {
        // `--no-cache` doesn't respect code changes, and so is often confusing during
        // development.
        nuff_linter::warn_user!("Detected debug build without --no-cache.");
    }

    let printer = Printer::new(
        output_format,
        config_arguments.log_level,
        fix_mode,
        unsafe_fixes,
        printer_flags,
    );

    let preview = pyproject_config.settings.linter.preview;
    let prefer_rule_codes = pyproject_config.settings.output_prefer_rule_codes;

    // Generate lint violations.
    let diagnostics = if is_stdin {
        commands::check_stdin::check_stdin(
            cli.stdin_filename.map(fs::normalize_path).as_deref(),
            &pyproject_config,
            &config_arguments,
            noqa.into(),
            fix_mode,
        )?
    } else {
        commands::check::check(
            &files,
            &pyproject_config,
            &config_arguments,
            cache.into(),
            noqa.into(),
            fix_mode,
            unsafe_fixes,
        )?
    };

    // Always try to print violations (though the printer itself may suppress output)
    // If we're writing fixes via stdin, the transformed source code goes to the writer
    // so send the summary to stderr instead
    let mut summary_writer = if is_stdin && matches!(fix_mode, FixMode::Apply | FixMode::Diff) {
        stderr_writer
    } else {
        writer
    };
    if cli.statistics {
        printer.write_statistics(&diagnostics, &mut summary_writer)?;
    } else {
        printer.write_once(
            &diagnostics,
            &mut summary_writer,
            preview,
            prefer_rule_codes,
        )?;
    }

    if !cli.exit_zero {
        let max_severity = diagnostics
            .inner
            .iter()
            .map(Diagnostic::severity)
            .max()
            .unwrap_or(Severity::Info);
        if max_severity.is_fatal() {
            // When a panic/fatal error is reported, prompt the user to open an issue on github.
            // Diagnostics with severity `fatal` will be sorted to the bottom, and printing the
            // message here instead of attaching it to the diagnostic ensures that we only print
            // it once instead of repeating it for each diagnostic. Prints to stderr to prevent
            // the message from being captured by tools parsing the normal output.
            error!("Panic during linting indicates a bug in nuff.");
            return Ok(ExitStatus::Error);
        }
        if cli.diff {
            // If we're printing a diff, we always want to exit non-zero if there are
            // any fixable violations (since we've printed the diff, but not applied the
            // fixes).
            if !diagnostics.fixed.is_empty() {
                return Ok(ExitStatus::Failure);
            }
        } else if fix_only {
            // If we're only fixing, we want to exit zero (since we've fixed all fixable
            // violations), unless we're explicitly asked to exit non-zero on fix.
            if cli.exit_non_zero_on_fix {
                if !diagnostics.fixed.is_empty() {
                    return Ok(ExitStatus::Failure);
                }
            }
        } else {
            // If we're running the linter (not just fixing), we want to exit non-zero if
            // there are any violations, unless we're explicitly asked to exit zero on
            // fix.
            if cli.exit_non_zero_on_fix {
                if !diagnostics.fixed.is_empty() || !diagnostics.inner.is_empty() {
                    return Ok(ExitStatus::Failure);
                }
            } else {
                if !diagnostics.inner.is_empty() {
                    return Ok(ExitStatus::Failure);
                }
            }
        }
    }
    Ok(ExitStatus::Success)
}

fn colored_override(
    color: Option<TerminalColor>,
    env_force_color: Option<OsString>,
) -> Option<bool> {
    match color {
        // Cli arguments should take precedence over env vars.
        Some(TerminalColor::Always) => Some(true),
        Some(TerminalColor::Never) => Some(false),
        // Default to no override, but respect FORCE_COLOR.
        Some(TerminalColor::Auto) | None => {
            // support FORCE_COLOR env var
            env_force_color.map(|force_color: OsString| !force_color.is_empty())
        }
    }
}

#[cfg(test)]
mod test_set_colored_override {
    use crate::{args::TerminalColor, colored_override};

    #[test]
    fn force_color_env_is_respected() {
        assert_eq!(colored_override(None, Some("1".into())), Some(true));
    }

    #[test]
    fn cli_args_takes_precedences_over_force_color_env() {
        assert_eq!(
            colored_override(Some(TerminalColor::Never), Some("1".into())),
            Some(false)
        );

        assert_eq!(
            colored_override(Some(TerminalColor::Always), None),
            Some(true)
        );
    }
}
