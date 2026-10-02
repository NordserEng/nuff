use nuff_macros::{ViolationMetadata, derive_message_formats};
use nuff_python_ast::ExprCall;
use nuff_text_size::Ranged;

use crate::Violation;
use crate::checkers::ast::Checker;
use crate::codes::Category;

/// ## What it does
/// Checks for an `async` function that calls a configured blocking function directly instead of
/// through `asyncio.to_thread`.
///
/// ## Why is this bad?
/// A synchronous call that waits on the network, the disk or the CPU stalls the event loop, and with
/// it every request the process is serving. `flake8-async` catches the standard library's blocking
/// calls written in the async function itself; this rule catches the project's own helpers that block
/// further down, which only the project can name.
///
/// ## Example
/// ```python
/// from app.platform.email import send_email
///
///
/// async def invite(email: str) -> None:
///     send_email(email, "Welcome")
/// ```
///
/// Use instead:
/// ```python
/// import asyncio
///
/// from app.platform.email import send_email
///
///
/// async def invite(email: str) -> None:
///     await asyncio.to_thread(send_email, email, "Welcome")
/// ```
///
/// ## Options
/// - `lint.nuff.blocking-functions`
#[derive(ViolationMetadata)]
#[violation_metadata(stable_since = "0.2.0", category = Category::Restriction)]
pub(crate) struct BlockingCallOutsideThread {
    name: String,
}

impl Violation for BlockingCallOutsideThread {
    #[derive_message_formats]
    fn message(&self) -> String {
        let BlockingCallOutsideThread { name } = self;
        format!("`{name}` blocks the event loop; await it through `asyncio.to_thread`")
    }
}

/// NUF001
pub(crate) fn blocking_call_outside_thread(checker: &Checker, call: &ExprCall) {
    let blocking = &checker.settings().nuff.blocking_functions;
    if blocking.is_empty() || !checker.semantic().in_async_context() {
        return;
    }
    let Some(qualified_name) = checker.semantic().resolve_qualified_name(&call.func) else {
        return;
    };
    let name = qualified_name.to_string();
    if blocking.contains(&name) {
        checker.report_diagnostic(BlockingCallOutsideThread { name }, call.func.range());
    }
}
