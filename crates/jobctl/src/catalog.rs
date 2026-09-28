use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use serde::de::DeserializeOwned;
use serde::{Serialize};
use serde_json::Value;

use crate::ctx::JobCtx;
use crate::services::Services;
use crate::EngineError;

pub(crate) type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Metadata for one catalogue entry.
#[derive(Debug, Clone)]
pub struct JobInfo {
    pub kind: &'static str,
    pub description: &'static str,
}

/// One entry in the job catalogue: self-describing, self-executing.
pub trait JobDefinition: Send + Sync + 'static {
    /// Unique catalogue key, e.g. `"ensure_vm"`.
    const KIND: &'static str;
    /// Human-readable one-liner shown by catalogue listings.
    const DESCRIPTION: &'static str;

    type Payload: DeserializeOwned + Send + 'static;
    type Event: Serialize + Send + 'static;
    type Output: Serialize + Send + 'static;

    fn run(
        ctx: JobCtx,
        payload: Self::Payload,
        services: Services,
    ) -> impl Future<Output = Result<Self::Output, EngineError>> + Send;
}

pub(crate) trait ErasedJob: Send + Sync {
    fn info(&self) -> JobInfo;
    fn run(&self, ctx: JobCtx, payload: Value, services: Services) -> Boxed<Result<Value, EngineError>>;
}

pub(crate) struct JobAdapter<D: JobDefinition>(PhantomData<D>);

impl<D: JobDefinition> JobAdapter<D> {
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}

impl<D: JobDefinition> ErasedJob for JobAdapter<D> {
    fn info(&self) -> JobInfo {
        JobInfo {
            kind: D::KIND,
            description: D::DESCRIPTION,
        }
    }

    fn run(&self, ctx: JobCtx, payload: Value, services: Services) -> Boxed<Result<Value, EngineError>> {
        Box::pin(async move {
            let payload: D::Payload = serde_json::from_value(payload)?;
            let output = D::run(ctx, payload, services).await?;
            Ok(serde_json::to_value(output)?)
        })
    }
}