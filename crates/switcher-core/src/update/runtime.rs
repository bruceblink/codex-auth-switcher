use super::UpdateError;
use std::{future::Future, sync::OnceLock};
use tokio::runtime::{Builder, Runtime};

fn runtime() -> Result<&'static Runtime, UpdateError> {
    static UPDATE_RUNTIME: OnceLock<Result<Runtime, UpdateError>> = OnceLock::new();
    match UPDATE_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("codex-auth-switcher-update")
            .enable_all()
            .build()
            .map_err(|_| UpdateError::RuntimeUnavailable)
    }) {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(*error),
    }
}

pub(super) async fn spawn<F, T>(future: F) -> Result<T, UpdateError>
where
    F: Future<Output = Result<T, UpdateError>> + Send + 'static,
    T: Send + 'static,
{
    runtime()?
        .spawn(future)
        .await
        .map_err(|_| UpdateError::RuntimeTask)?
}
