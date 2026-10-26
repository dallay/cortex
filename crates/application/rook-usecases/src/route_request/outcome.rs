use super::CortexError;

/// Result category for one upstream attempt.
///
/// This keeps local admission pressure distinct from an upstream failure so
/// callers can release a half-open reservation without changing circuit state.
pub(super) enum AttemptOutcome<T> {
    Success(T),
    LocalBackpressure(CortexError),
    UpstreamFailure(CortexError),
    TimedOut,
}

pub(super) enum AttemptResult<T> {
    Provider(Result<T, CortexError>),
    TimedOut,
}

pub(super) fn classify_attempt_result<T>(result: AttemptResult<T>) -> AttemptOutcome<T> {
    match result {
        AttemptResult::Provider(Ok(value)) => AttemptOutcome::Success(value),
        AttemptResult::Provider(Err(error)) if error.is_local_backpressure() => {
            AttemptOutcome::LocalBackpressure(error)
        }
        AttemptResult::Provider(Err(error)) => AttemptOutcome::UpstreamFailure(error),
        AttemptResult::TimedOut => AttemptOutcome::TimedOut,
    }
}

#[cfg(test)]
mod tests {
    use super::super::CortexError;
    use super::{classify_attempt_result, AttemptOutcome, AttemptResult};

    #[test]
    fn classifies_success_local_backpressure_upstream_failure_and_timeout() {
        assert!(matches!(
            classify_attempt_result(AttemptResult::Provider(Ok(()))),
            AttemptOutcome::Success(())
        ));

        assert!(matches!(
            classify_attempt_result::<()>(AttemptResult::Provider(Err(
                CortexError::local_backpressure()
            ))),
            AttemptOutcome::LocalBackpressure(error) if error.is_local_backpressure()
        ));

        assert!(matches!(
            classify_attempt_result::<()>(AttemptResult::Provider(Err(CortexError::provider(
                "upstream failed"
            )))),
            AttemptOutcome::UpstreamFailure(error) if error.is_retryable()
        ));

        assert!(matches!(
            classify_attempt_result::<()>(AttemptResult::TimedOut),
            AttemptOutcome::TimedOut
        ));
    }
}
