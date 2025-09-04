#[cfg(not(target_arch = "wasm32"))]
pub trait NativeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> NativeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait NativeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> NativeSend for T {}
