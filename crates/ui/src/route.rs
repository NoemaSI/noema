#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    NotLoggedIn,
    Workspace,
    Settings,
}

impl Default for Route {
    fn default() -> Self {
        Self::Workspace
    }
}
