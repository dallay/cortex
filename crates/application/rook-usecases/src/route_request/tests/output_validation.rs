use super::*;

/// Each stream must observe exactly its own two upstream chunks before EOF.
struct TwoChunkPolicy;

impl OutputValidation for TwoChunkPolicy {
    fn response(&self, _: &CompletionResponse) -> CortexResult<()> {
        Err(CortexError::provider("local response rejection"))
    }

    fn stream(&self) -> Box<dyn StreamOutputValidation> {
        Box::new(TwoChunkValidator { seen: 0 })
    }
}

struct TwoChunkValidator {
    seen: usize,
}

impl StreamOutputValidation for TwoChunkValidator {
    fn chunk(&mut self, _: &StreamChunk) -> CortexResult<()> {
        self.seen += 1;
        if self.seen > 2 {
            return Err(CortexError::provider("state leaked from another stream"));
        }
        Ok(())
    }

    fn finish(&mut self) -> CortexResult<()> {
        if self.seen != 2 {
            return Err(CortexError::provider("missing chunks at EOF"));
        }
        Ok(())
    }
}

struct RejectAtEof;

impl OutputValidation for RejectAtEof {
    fn response(&self, _: &CompletionResponse) -> CortexResult<()> {
        Ok(())
    }

    fn stream(&self) -> Box<dyn StreamOutputValidation> {
        Box::new(Self)
    }
}

impl StreamOutputValidation for RejectAtEof {
    fn chunk(&mut self, _: &StreamChunk) -> CortexResult<()> {
        Ok(())
    }

    fn finish(&mut self) -> CortexResult<()> {
        Err(CortexError::provider("local EOF rejection"))
    }
}

#[tokio::test]
async fn interleaved_streams_on_cloned_routes_have_independent_validation_state() {
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(vec![]),
    });
    let mut usecase = make_usecase().with_output_validation(Arc::new(TwoChunkPolicy));
    usecase.audit = audit.clone();
    let clone = usecase.clone();
    let mut first = usecase.execute_stream(request()).await.unwrap();
    let mut second = clone.execute_stream(request()).await.unwrap();
    for expected in ["hel", "lo"] {
        assert_eq!(first.next().await.unwrap().unwrap().delta, expected);
        assert_eq!(second.next().await.unwrap().unwrap().delta, expected);
    }
    assert!(first.next().await.is_none());
    assert!(second.next().await.is_none());
    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries
        .iter()
        .all(|entry| entry.status == RequestStatus::Success));
}

#[tokio::test]
async fn dropping_one_validated_stream_does_not_invalidate_a_sibling() {
    let usecase = make_usecase().with_output_validation(Arc::new(TwoChunkPolicy));
    let mut cancelled = usecase.execute_stream(request()).await.unwrap();
    let mut sibling = usecase.execute_stream(request()).await.unwrap();
    assert_eq!(cancelled.next().await.unwrap().unwrap().delta, "hel");
    assert_eq!(sibling.next().await.unwrap().unwrap().delta, "hel");
    drop(cancelled);
    assert_eq!(sibling.next().await.unwrap().unwrap().delta, "lo");
    assert!(sibling.next().await.is_none());
}

#[tokio::test]
async fn local_nonstream_rejection_is_accounted_once_and_never_cached() {
    let cache = Arc::new(TestCache {
        get_calls: Mutex::new(0),
        set_calls: Mutex::new(0),
    });
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(vec![]),
    });
    let recorder = Arc::new(TestUsageRecorder {
        entries: Mutex::new(vec![]),
        fail_recording: false,
    });
    let mut usecase = make_usecase().with_output_validation(Arc::new(TwoChunkPolicy));
    usecase.cache = cache.clone();
    usecase.audit = audit.clone();
    usecase.usage_recorder = Some(recorder.clone());
    let mut req = request();
    req.stream = false;
    let request_id = req.id.clone();
    let error = usecase.execute(req).await.unwrap_err();
    assert!(error.to_string().contains("local response rejection"));
    assert_eq!(*cache.set_calls.lock().unwrap(), 0);
    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Failure);
    assert_eq!(entries[0].request_id, request_id);
    assert_eq!(entries[0].usage.as_ref().unwrap().total_tokens, 2);
    let usage = recorder.entries.lock().unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].status, RequestStatus::Failure);
}

#[tokio::test]
async fn eof_rejection_preserves_observed_usage_without_poisoning_provider_health() {
    let router = Arc::new(crate::FallbackRouter::new(
        vec![ok_provider("test-provider")],
        crate::RoutingStrategy::Priority,
    ));
    let audit = Arc::new(TestAudit {
        entries: Mutex::new(vec![]),
    });
    let mut usecase =
        usecase_with_router(router.clone()).with_output_validation(Arc::new(RejectAtEof));
    usecase.audit = audit.clone();
    let mut stream = usecase.execute_stream(request()).await.unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "hel");
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "lo");
    let error = stream.next().await.unwrap().unwrap_err();
    assert!(error.to_string().contains("local EOF rejection"));
    assert!(stream.next().await.is_none());
    drop(stream);
    let entries = audit.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, RequestStatus::Failure);
    assert_eq!(entries[0].usage.as_ref().unwrap().total_tokens, 5);
    assert_eq!(router.circuit_states()[0].1.failures, 0);
}
