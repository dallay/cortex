//! Access to task ownership at the submission boundary.
use super::{HostWorkflow, PreparedSubmission};

pub fn prepare_submission(form: &mut HostWorkflow) -> Option<PreparedSubmission> {
    form.prepare_submission()
}

pub fn attach_task(form: &mut HostWorkflow, task: tokio::task::JoinHandle<()>) {
    form.attach_task(task);
}
