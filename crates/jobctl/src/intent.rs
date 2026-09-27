use std::future::Future;

use crate::{EngineError, Handler, JobId};

/// Job submitted when the user clicks "Analyze intent".
#[derive(Debug)]
pub struct IntentJob {
    pub name: String,
    pub goal: String,
}

/// Stub handler: returns the acceptance criteria the agent would propose.
/// Later this will call the real analysis backend.
pub struct IntentHandler;

impl Handler for IntentHandler {
    type Job = IntentJob;
    type Output = Vec<String>;

    fn handle(
        &self,
        _id: JobId,
        job: IntentJob,
    ) -> impl Future<Output = Result<Self::Output, EngineError>> + Send {
        async move {
            if job.goal.trim().is_empty() {
                return Err(EngineError::JobFailed(
                    "goal is empty; nothing to analyse".into(),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            Ok(vec![
                "fit converges".into(),
                "validated on held-out runs".into(),
                "report cites CIs".into(),
            ])
        }
    }
}
