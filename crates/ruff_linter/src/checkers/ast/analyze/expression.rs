use ruff_python_ast::{self as ast, Arguments, Expr, Operator};
use ruff_python_literal::cformat::{CFormatError, CFormatErrorType};
use ruff_text_size::Ranged;

use crate::checkers::ast::Checker;
use crate::registry::Rule;
use crate::rules::{flake8_async, nuff, pyflakes};

/// Run lint rules over an [`Expr`] syntax node.
pub(crate) fn expression(expr: &Expr, checker: &Checker) {
    match expr {
        Expr::Tuple(ast::ExprTuple { elts, ctx, .. })
        | Expr::List(ast::ExprList { elts, ctx, .. }) => {
            if ctx.is_store() {
                let check_too_many_expressions =
                    checker.is_rule_enabled(Rule::ExpressionsInStarAssignment);
                pyflakes::rules::starred_expressions(
                    checker,
                    elts,
                    check_too_many_expressions,
                    expr.range(),
                );
            }
        }
        Expr::Call(
            call @ ast::ExprCall {
                func,
                arguments: Arguments { args, keywords, .. },
                ..
            },
        ) => {
            if checker.any_rule_enabled(&[
                Rule::StringDotFormatInvalidFormat,
                Rule::StringDotFormatExtraNamedArguments,
                Rule::StringDotFormatExtraPositionalArguments,
                Rule::StringDotFormatMissingArguments,
                Rule::StringDotFormatMixingAutomatic,
            ]) {
                if let Expr::Attribute(ast::ExprAttribute { value, attr, .. }) = func.as_ref() {
                    let attr = attr.as_str();
                    if let Expr::StringLiteral(ast::ExprStringLiteral {
                        value: string_value,
                        ..
                    }) = value.as_ref()
                    {
                        if attr == "format" {
                            let location = expr.range();
                            match pyflakes::format::FormatSummary::try_from(string_value.to_str()) {
                                Err(e) => {
                                    // F521
                                    checker.report_diagnostic_if_enabled(
                                        pyflakes::rules::StringDotFormatInvalidFormat {
                                            message: pyflakes::format::error_to_string(&e),
                                        },
                                        location,
                                    );
                                }
                                Ok(summary) => {
                                    if checker
                                        .is_rule_enabled(Rule::StringDotFormatExtraNamedArguments)
                                    {
                                        pyflakes::rules::string_dot_format_extra_named_arguments(
                                            checker, call, &summary, keywords,
                                        );
                                    }
                                    if checker.is_rule_enabled(
                                        Rule::StringDotFormatExtraPositionalArguments,
                                    ) {
                                        pyflakes::rules::string_dot_format_extra_positional_arguments(
                                            checker, call, &summary, args,
                                        );
                                    }
                                    if checker
                                        .is_rule_enabled(Rule::StringDotFormatMissingArguments)
                                    {
                                        pyflakes::rules::string_dot_format_missing_argument(
                                            checker, call, &summary, args, keywords,
                                        );
                                    }
                                    if checker.is_rule_enabled(Rule::StringDotFormatMixingAutomatic)
                                    {
                                        pyflakes::rules::string_dot_format_mixing_automatic(
                                            checker, call, &summary,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if checker.is_rule_enabled(Rule::BlockingHttpCallInAsyncFunction) {
                flake8_async::rules::blocking_http_call(checker, call);
            }
            if checker.is_rule_enabled(Rule::BlockingHttpCallHttpxInAsyncFunction) {
                flake8_async::rules::blocking_http_call_httpx(checker, call);
            }
            if checker.is_rule_enabled(Rule::BlockingOpenCallInAsyncFunction) {
                flake8_async::rules::blocking_open_call(checker, call);
            }
            if checker.is_rule_enabled(Rule::BlockingPathMethodInAsyncFunction) {
                flake8_async::rules::blocking_os_path(checker, call);
            }
            if checker.any_rule_enabled(&[
                Rule::CreateSubprocessInAsyncFunction,
                Rule::RunProcessInAsyncFunction,
                Rule::WaitForProcessInAsyncFunction,
            ]) {
                flake8_async::rules::blocking_process_invocation(checker, call);
            }
            if checker.is_rule_enabled(Rule::BlockingInputInAsyncFunction) {
                flake8_async::rules::blocking_input(checker, call);
            }
            if checker.is_rule_enabled(Rule::BlockingSleepInAsyncFunction) {
                flake8_async::rules::blocking_sleep(checker, call);
            }
            if checker.is_rule_enabled(Rule::BlockingCallOutsideThread) {
                nuff::rules::blocking_call_outside_thread(checker, call);
            }
            if checker.is_rule_enabled(Rule::ForeignKeyModelNotImported) {
                nuff::rules::foreign_key_model_not_imported(checker, call);
            }
            if checker.is_rule_enabled(Rule::LongSleepNotForever) {
                flake8_async::rules::long_sleep_not_forever(checker, call);
            }
            if checker.is_rule_enabled(Rule::TrioSyncCall) {
                flake8_async::rules::sync_call(checker, call);
            }
            if checker.is_rule_enabled(Rule::AsyncZeroSleep) {
                flake8_async::rules::async_zero_sleep(checker, call);
            }
        }
        Expr::Dict(dict) => {
            if checker.any_rule_enabled(&[
                Rule::MultiValueRepeatedKeyLiteral,
                Rule::MultiValueRepeatedKeyVariable,
            ]) {
                pyflakes::rules::repeated_keys(checker, dict);
            }
        }
        Expr::Yield(_) => {
            if checker.is_rule_enabled(Rule::YieldInContextManagerInAsyncGenerator) {
                flake8_async::rules::yield_in_context_manager_in_async_generator(checker, expr);
            }
        }
        Expr::FString(f_string_expr) => {
            if checker.is_rule_enabled(Rule::FStringMissingPlaceholders) {
                pyflakes::rules::f_string_missing_placeholders(checker, f_string_expr);
            }
        }
        Expr::BinOp(ast::ExprBinOp {
            left,
            op: Operator::RShift,
            ..
        }) => {
            if checker.is_rule_enabled(Rule::InvalidPrintSyntax) {
                pyflakes::rules::invalid_print_syntax(checker, left);
            }
        }
        Expr::BinOp(ast::ExprBinOp {
            left,
            op: Operator::Mod,
            right,
            range: _,
            node_index: _,
        }) => {
            if let Expr::StringLiteral(_format_string @ ast::ExprStringLiteral { value, .. }) =
                left.as_ref()
            {
                if checker.any_rule_enabled(&[
                    Rule::PercentFormatInvalidFormat,
                    Rule::PercentFormatExpectedMapping,
                    Rule::PercentFormatExpectedSequence,
                    Rule::PercentFormatExtraNamedArguments,
                    Rule::PercentFormatMissingArgument,
                    Rule::PercentFormatMixedPositionalAndNamed,
                    Rule::PercentFormatPositionalCountMismatch,
                    Rule::PercentFormatStarRequiresSequence,
                    Rule::PercentFormatUnsupportedFormatCharacter,
                ]) {
                    let location = expr.range();
                    match pyflakes::cformat::CFormatSummary::try_from(value.to_str()) {
                        Err(CFormatError {
                            typ: CFormatErrorType::UnsupportedFormatChar(c),
                            ..
                        }) => {
                            // F509
                            checker.report_diagnostic_if_enabled(
                                pyflakes::rules::PercentFormatUnsupportedFormatCharacter {
                                    char: c,
                                },
                                location,
                            );
                        }
                        Err(e) => {
                            // F501
                            checker.report_diagnostic_if_enabled(
                                pyflakes::rules::PercentFormatInvalidFormat {
                                    message: e.to_string(),
                                },
                                location,
                            );
                        }
                        Ok(summary) => {
                            if checker.is_rule_enabled(Rule::PercentFormatExpectedMapping) {
                                pyflakes::rules::percent_format_expected_mapping(
                                    checker, &summary, right, location,
                                );
                            }
                            if checker.is_rule_enabled(Rule::PercentFormatExpectedSequence) {
                                pyflakes::rules::percent_format_expected_sequence(
                                    checker, &summary, right, location,
                                );
                            }
                            if checker.is_rule_enabled(Rule::PercentFormatExtraNamedArguments) {
                                pyflakes::rules::percent_format_extra_named_arguments(
                                    checker, &summary, right, location,
                                );
                            }
                            if checker.is_rule_enabled(Rule::PercentFormatMissingArgument) {
                                pyflakes::rules::percent_format_missing_arguments(
                                    checker, &summary, right, location,
                                );
                            }
                            if checker.is_rule_enabled(Rule::PercentFormatMixedPositionalAndNamed) {
                                pyflakes::rules::percent_format_mixed_positional_and_named(
                                    checker, &summary, location,
                                );
                            }
                            if checker.is_rule_enabled(Rule::PercentFormatPositionalCountMismatch) {
                                pyflakes::rules::percent_format_positional_count_mismatch(
                                    checker, &summary, right, location,
                                );
                            }
                            if checker.is_rule_enabled(Rule::PercentFormatStarRequiresSequence) {
                                pyflakes::rules::percent_format_star_requires_sequence(
                                    checker, &summary, right, location,
                                );
                            }
                        }
                    }
                }
            }
        }
        Expr::Compare(compare) => {
            if checker.is_rule_enabled(Rule::IsLiteral) {
                pyflakes::rules::invalid_literal_comparison(checker, compare);
            }
        }
        _ => {}
    }
}
