//! Router-level behavior that does not need a graph database.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::{router, BridgeState};
use muninn_core::model::{Conversation, Message, Metadata, SessionInfo};
use muninn_core::ports::{
    GraphMaintenance, MessageRecord, ScoredMessage, SessionQuery, ShortTermStore,
};
use muninn_core::MemoryResult;
use tower::ServiceExt;
use uuid::Uuid;

/// Store that must never be reached by these tests.
struct UnreachableStore;

#[async_trait]
impl ShortTermStore for UnreachableStore {
    async fn ensure_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        _: Option<&str>,
    ) -> MemoryResult<Uuid> {
        unreachable!()
    }
    async fn append_message(&self, _: Uuid, _: &MessageRecord) -> MemoryResult<Message> {
        unreachable!()
    }
    async fn append_messages(&self, _: Uuid, _: &[MessageRecord]) -> MemoryResult<Vec<Message>> {
        unreachable!()
    }
    async fn get_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        _: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        unreachable!()
    }
    async fn get_conversation_by_id(&self, _: Uuid) -> MemoryResult<Option<Conversation>> {
        unreachable!()
    }
    async fn list_conversations(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Conversation>> {
        unreachable!()
    }
    async fn search_messages(
        &self,
        _: &[f64],
        _: Option<&str>,
        _: i64,
        _: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        unreachable!()
    }
    async fn list_sessions(&self, _: &SessionQuery) -> MemoryResult<Vec<SessionInfo>> {
        unreachable!()
    }
    async fn delete_message(&self, _: Uuid, _: bool) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn clear_session(&self, _: &str) -> MemoryResult<()> {
        unreachable!()
    }
    async fn delete_conversation(&self, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
}

#[derive(Default)]
struct RecordingMaintenance(AtomicBool);

#[async_trait]
impl GraphMaintenance for RecordingMaintenance {
    async fn clear_all_data(&self) -> MemoryResult<()> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn state(maintenance: Option<Arc<dyn GraphMaintenance>>) -> Arc<BridgeState> {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore),
            embedder: None,
        },
        &MemoryOptions::default(),
    );
    Arc::new(BridgeState {
        memory: Arc::new(memory),
        maintenance,
    })
}

async fn post(state: Arc<BridgeState>, method: &str, body: &str) -> StatusCode {
    router(state)
        .oneshot(
            Request::post(format!("/{method}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn clear_all_data_is_refused_unless_explicitly_enabled() {
    assert_eq!(
        post(state(None), "clear_all_data", "{}").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn clear_all_data_wipes_when_enabled() {
    let maintenance = Arc::new(RecordingMaintenance::default());
    let status = post(state(Some(maintenance.clone())), "clear_all_data", "{}").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(maintenance.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn unknown_methods_are_rejected() {
    assert_eq!(
        post(state(None), "drop_database", "{}").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn missing_arguments_are_bad_requests() {
    assert_eq!(
        post(state(None), "add_message", "{}").await,
        StatusCode::BAD_REQUEST
    );
}
