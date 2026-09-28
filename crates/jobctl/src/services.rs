use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Type-map of shared singletons (VM handle, HTTP client, ...) built at
/// `EngineBuilder::service(...)` time and handed to every job.
#[derive(Clone, Default)]
pub struct Services {
    map: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Services {
    pub(crate) fn new(map: HashMap<TypeId, Arc<dyn Any + Send + Sync>>) -> Self {
        Self { map: Arc::new(map) }
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.map
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|svc| svc.downcast().ok())
    }
}