//! Propietario independiente: nunca cancela un intercambio Bolt en curso.
use muninn_core::{MemoryError, MemoryResult};
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{oneshot, Semaphore},
    time::Instant,
};

const RESPONSE_BUDGET: Duration = Duration::from_secs(5);
static ADMISSION: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub struct Control<T> {
    sender: oneshot::Sender<MemoryResult<T>>,
    deadline: Instant,
}
impl<T> Control<T> {
    pub(crate) fn check(&self) -> MemoryResult<()> {
        if self.sender.is_closed() || Instant::now() >= self.deadline {
            Err(MemoryError::storage(
                "preference consumer cancelled or response deadline elapsed",
            ))
        } else {
            Ok(())
        }
    }
}

pub async fn run<T, F, Fut>(operation: F) -> MemoryResult<T>
where
    T: Send + 'static,
    F: FnOnce(Control<T>) -> Fut + Send + 'static,
    Fut: Future<Output = (Control<T>, MemoryResult<T>)> + Send + 'static,
{
    run_with(
        ADMISSION
            .get_or_init(|| Arc::new(Semaphore::new(16)))
            .clone(),
        RESPONSE_BUDGET,
        operation,
    )
    .await
}

async fn run_with<T, F, Fut>(
    admission: Arc<Semaphore>,
    budget: Duration,
    operation: F,
) -> MemoryResult<T>
where
    T: Send + 'static,
    F: FnOnce(Control<T>) -> Fut + Send + 'static,
    Fut: Future<Output = (Control<T>, MemoryResult<T>)> + Send + 'static,
{
    let deadline = Instant::now() + budget;
    let permit = admission
        .try_acquire_owned()
        .map_err(|_| MemoryError::storage("preference writer admission exhausted"))?;
    let (sender, receiver) = oneshot::channel();
    // No se aborta esta tarea al cancelar el consumidor. El permiso permanece
    // retenido hasta finalizar COMMIT/ROLLBACK, incluso después del plazo.
    tokio::spawn(async move {
        let _permit = permit;
        let (control, result) = operation(Control { sender, deadline }).await;
        let _ = control.sender.send(result);
    });
    tokio::time::timeout_at(deadline, receiver)
        .await
        .map_err(|_| {
            MemoryError::storage(
                "preference response deadline elapsed; outcome unknown; do not retry blindly",
            )
        })?
        .map_err(|_| MemoryError::storage("preference owner stopped; outcome unknown"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_keeps_owner_and_admission_until_exchange_and_cleanup_finish() {
        let admission = Arc::new(Semaphore::new(1));
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let caller_admission = admission.clone();
        let caller = tokio::spawn(run_with(
            caller_admission,
            Duration::from_secs(2),
            move |control| async move {
                entered_tx.send(()).unwrap();
                release_rx.await.unwrap(); // Intercambio en curso: no se cancela.
                assert!(control.check().is_err());
                finished_tx.send(()).unwrap();
                (control, Ok(()))
            },
        ));
        entered_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert_eq!(admission.available_permits(), 0);
        let refused = run_with(admission.clone(), Duration::from_secs(1), |control| async {
            (control, Ok(()))
        })
        .await;
        assert!(refused
            .unwrap_err()
            .to_string()
            .contains("admission exhausted"));
        release_tx.send(()).unwrap();
        finished_rx.await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while admission.available_permits() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn deadline_bounds_response_without_cancelling_owner_or_claiming_rollback() {
        let admission = Arc::new(Semaphore::new(1));
        let (release_tx, release_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let error = run_with(
            admission.clone(),
            Duration::from_millis(10),
            |control| async move {
                release_rx.await.unwrap();
                assert!(control.check().is_err());
                finished_tx.send(()).unwrap();
                (control, Ok(()))
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("outcome unknown"));
        assert_eq!(admission.available_permits(), 0);
        release_tx.send(()).unwrap();
        finished_rx.await.unwrap();
    }
}
