//! Trap / exit / protocol error mapping for the native wasmtime archive host
//! (RFC 123 §7.2.8).
//!
//! `AppError` has no dedicated timeout/resource-limit/protocol variant, so every
//! failure category maps to `AppError::Repository` with a distinct, categorized
//! message — matching the existing archive path, which also used `Repository`,
//! and keeping the change out of the middleware/GraphQL error surface. The
//! category is captured in the testable `FailureKind` enum before it is
//! flattened into the message.

use scryer_application::AppError;
use wasmtime::Trap;
use wasmtime_wasi::I32Exit;

/// The §7.2.8 error categories, kept as a discriminated value so the
/// classification logic can be unit-tested without a running guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    /// Epoch deadline fired — the guest exceeded its wall-clock budget.
    Timeout,
    /// The store limiter denied a memory allocation (cap exceeded / OOM).
    ResourceLimit,
    /// Non-zero `proc_exit`, or any other trap: a guest-side fault.
    PluginFailure,
    /// The guest exited cleanly but produced malformed / absent stdout JSON.
    Protocol,
}

/// A classified invocation failure, before message formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunFailure {
    pub(crate) kind: FailureKind,
    /// Category-specific detail (exit code, trap text, parse error, …).
    pub(crate) detail: String,
}

impl RunFailure {
    fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// Context threaded into the operator-facing message.
pub(crate) struct InvocationContext<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
    /// Wall-clock budget for the run (names the timeout in the message).
    pub(crate) budget: std::time::Duration,
    /// Size-capped tail of guest stderr, if any.
    pub(crate) stderr_tail: &'a str,
}

/// Interpret the result of `TypedFunc::call(_start)`.
///
/// Returns `Ok(())` for a clean return or `proc_exit(0)` (both are protocol
/// success — the response is then read from stdout). Any other disposition is
/// classified into a [`RunFailure`]. `memory_denied` reflects the store
/// limiter: a denied growth is reported as a resource limit regardless of the
/// symptomatic trap, because that is the actionable cause.
pub(crate) fn interpret_start_result(
    result: Result<(), wasmtime::Error>,
    memory_denied: bool,
) -> Result<(), RunFailure> {
    match result {
        // Clean dispositions (return or `proc_exit(0)`) are success ONLY if the
        // limiter recorded no denial: a guest that survived a denied allocation
        // and still exited clean cannot have completed correctly, so the denial
        // is the actionable cause (mirrors `classify_error`'s precedence).
        Ok(()) if !memory_denied => Ok(()),
        Err(ref error)
            if !memory_denied
                && error
                    .downcast_ref::<I32Exit>()
                    .is_some_and(|exit| exit.0 == 0) =>
        {
            Ok(())
        }
        Ok(()) => Err(RunFailure::new(
            FailureKind::ResourceLimit,
            "guest exceeded its memory cap and could not complete",
        )),
        Err(error) => Err(classify_error(&error, memory_denied)),
    }
}

/// Classify a wasmtime error raised during instantiation or the `_start` call.
///
/// `memory_denied` is checked first: a limiter denial surfaces downstream as a
/// trap (or as a non-zero exit once the guest aborts), but the resource limit is
/// the root cause we want to report.
pub(crate) fn classify_error(error: &wasmtime::Error, memory_denied: bool) -> RunFailure {
    if memory_denied {
        return RunFailure::new(
            FailureKind::ResourceLimit,
            "guest exceeded the configured memory cap",
        );
    }
    if let Some(exit) = error.downcast_ref::<I32Exit>() {
        return RunFailure::new(
            FailureKind::PluginFailure,
            format!("guest exited with status {}", exit.0),
        );
    }
    if let Some(trap) = error.downcast_ref::<Trap>() {
        if *trap == Trap::Interrupt {
            return RunFailure::new(FailureKind::Timeout, "epoch deadline exceeded");
        }
        return RunFailure::new(FailureKind::PluginFailure, format!("guest trapped: {trap}"));
    }
    RunFailure::new(FailureKind::PluginFailure, format!("{error:#}"))
}

/// A malformed / absent stdout response after an otherwise clean run.
pub(crate) fn protocol_failure(detail: impl Into<String>) -> RunFailure {
    RunFailure::new(FailureKind::Protocol, detail)
}

/// A host-side wall-clock overrun (e.g. the PAR2 reconstruct deadline) that the
/// engine epoch could not catch because the work ran synchronously on the host
/// thread. Classified as a timeout so it maps to the same operator-facing
/// "timed out" message as an epoch interrupt.
pub(crate) fn timeout_failure(detail: impl Into<String>) -> RunFailure {
    RunFailure::new(FailureKind::Timeout, detail)
}

/// Flatten a classified failure into the operator-facing `AppError`.
pub(crate) fn to_app_error(failure: &RunFailure, ctx: &InvocationContext<'_>) -> AppError {
    let plugin = format!("{}@{}", ctx.plugin_id, ctx.plugin_version);
    let stderr = if ctx.stderr_tail.trim().is_empty() {
        String::new()
    } else {
        format!(" (stderr: {})", ctx.stderr_tail.trim())
    };
    let message = match failure.kind {
        FailureKind::Timeout => format!(
            "archive extractor plugin {plugin} timed out during {} after {:?}: {}",
            ctx.operation, ctx.budget, failure.detail
        ),
        FailureKind::ResourceLimit => format!(
            "archive extractor plugin {plugin} exceeded its memory limit during {}: {}{stderr}",
            ctx.operation, failure.detail
        ),
        FailureKind::PluginFailure => format!(
            "archive extractor plugin {plugin} failed during {}: {}{stderr}",
            ctx.operation, failure.detail
        ),
        FailureKind::Protocol => format!(
            "archive extractor plugin {plugin} returned a malformed response during {}: {}{stderr}",
            ctx.operation, failure.detail
        ),
    };
    AppError::Repository(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_return_is_success() {
        assert_eq!(interpret_start_result(Ok(()), false), Ok(()));
    }

    #[test]
    fn proc_exit_zero_is_success() {
        let err = wasmtime::Error::from(I32Exit(0));
        assert_eq!(interpret_start_result(Err(err), false), Ok(()));
    }

    #[test]
    fn nonzero_exit_is_plugin_failure_with_code() {
        let err = wasmtime::Error::from(I32Exit(3));
        let failure = interpret_start_result(Err(err), false).unwrap_err();
        assert_eq!(failure.kind, FailureKind::PluginFailure);
        assert!(
            failure.detail.contains('3'),
            "detail should name the exit code: {}",
            failure.detail
        );
    }

    #[test]
    fn epoch_interrupt_is_timeout() {
        let err = wasmtime::Error::from(Trap::Interrupt);
        let failure = interpret_start_result(Err(err), false).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Timeout);
    }

    #[test]
    fn memory_denied_wins_over_symptom() {
        // Even if the symptom is a generic trap, a limiter denial reports the
        // resource limit as the root cause.
        let err = wasmtime::Error::from(Trap::MemoryOutOfBounds);
        let failure = interpret_start_result(Err(err), true).unwrap_err();
        assert_eq!(failure.kind, FailureKind::ResourceLimit);

        // ...and even a clean return or a zero exit is superseded by a recorded
        // denial through the production path itself (not just `classify_error`).
        assert_eq!(
            interpret_start_result(Ok(()), true).unwrap_err().kind,
            FailureKind::ResourceLimit
        );
        assert_eq!(
            interpret_start_result(Err(wasmtime::Error::from(I32Exit(0))), true)
                .unwrap_err()
                .kind,
            FailureKind::ResourceLimit
        );
        // A clean disposition with no denial stays success.
        assert!(interpret_start_result(Ok(()), false).is_ok());
    }

    #[test]
    fn other_trap_is_plugin_failure() {
        let err = wasmtime::Error::from(Trap::UnreachableCodeReached);
        let failure = interpret_start_result(Err(err), false).unwrap_err();
        assert_eq!(failure.kind, FailureKind::PluginFailure);
    }

    #[test]
    fn generic_error_is_plugin_failure() {
        let err = wasmtime::Error::msg("module has no start function");
        let failure = classify_error(&err, false);
        assert_eq!(failure.kind, FailureKind::PluginFailure);
    }

    #[test]
    fn app_error_messages_are_categorized() {
        let ctx = InvocationContext {
            plugin_id: "com.scryer.archive",
            plugin_version: "1.2.3",
            operation: "ExtractArchive",
            budget: std::time::Duration::from_secs(3600),
            stderr_tail: "boom",
        };
        let timeout = to_app_error(
            &RunFailure::new(FailureKind::Timeout, "epoch deadline exceeded"),
            &ctx,
        );
        let AppError::Repository(message) = timeout else {
            panic!("expected Repository");
        };
        assert!(message.contains("timed out"));
        assert!(message.contains("com.scryer.archive@1.2.3"));

        let protocol = to_app_error(&protocol_failure("expected value at line 1"), &ctx);
        let AppError::Repository(message) = protocol else {
            panic!("expected Repository");
        };
        assert!(message.contains("malformed response"));
        assert!(message.contains("stderr: boom"));
    }
}
