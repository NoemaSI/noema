//! Handler catalogue for the guest agent server.

use std::future::Future;

use crate::server::{AgentHandler, Emit};
use crate::{AgentEvent, AgentRequest};

/// Stub goal run: pretends to analyse, plan, and execute.
/// rig + opencode (A2A) will drive the real events here later.
pub struct GoalRun;

impl AgentHandler for GoalRun {
    const NAME: &'static str = crate::GOAL_HANDLER;
    const DESCRIPTION: &'static str = "Execute the skill goal (stub; rig + opencode later)";

    type Payload = AgentRequest;

    fn run(
        _payload: Self::Payload,
        emit: Emit,
    ) -> impl Future<Output = Result<String, String>> + Send {
        async move {
            // Stub intent analysis: the agent derives acceptance criteria
            // from the goal. rig + opencode will replace this whole body.
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            emit.emit(AgentEvent::Criteria {
                items: vec![
                    "fit converges".into(),
                    "validated on held-out runs".into(),
                    "report cites CIs".into(),
                ],
            });
            Ok("stub run finished".into())
        }
    }
}
