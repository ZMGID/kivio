//! Runtime seam for the imported UI; all GPUI types come from Kit.
use gpui_kit::{App, Global, Task};
#[derive(Clone)]
pub struct Tokio(tokio::runtime::Handle);
impl Global for Tokio {}
pub fn init_from_handle(cx: &mut App, handle: tokio::runtime::Handle) {
    cx.set_global(Tokio(handle));
}
impl Tokio {
    pub fn handle(cx: &App) -> &tokio::runtime::Handle {
        &cx.global::<Self>().0
    }
    pub fn spawn<R: Send + 'static>(
        cx: &App,
        future: impl std::future::Future<Output = R> + Send + 'static,
    ) -> Task<Result<R, tokio::task::JoinError>> {
        let task = Self::handle(cx).spawn(future);
        cx.background_executor().spawn(task)
    }
}
