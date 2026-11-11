//! `!Send` page values behind `Send` handles (web plan S1), without `unsafe`.
//!
//! The browser build has one thread, but `Backend` and `TableCalls` are `Send + Sync`
//! (`#[async_trait]`), and `JsValue`, `JsFuture` and `Closure` are not. A [`Local<T>`] is a
//! key into a thread-local registry that owns the value: the handle is `Send + Sync`, the
//! value never leaves the page's thread, and dropping the handle frees it. [`on_page`] runs a
//! `!Send` future on the page's event loop and hands back a `Send` future of its output.
//!
//! (The plan sketched `unsafe impl Send` wrappers; the workspace forbids `unsafe_code`.)

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::oneshot;

thread_local! {
    static VALUES: RefCell<HashMap<u64, Rc<dyn Any>>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A `Send + Sync` handle to a page-thread value of type `T`.
pub struct Local<T: 'static> {
    key: u64,
    // `fn() -> T` keeps the handle `Send + Sync` whatever `T` is.
    _t: PhantomData<fn() -> T>,
}

impl<T: 'static> std::fmt::Debug for Local<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Local").field("key", &self.key).finish()
    }
}

impl<T: 'static> Local<T> {
    /// Stores `value` on this thread.
    pub fn new(value: T) -> Local<T> {
        let key = NEXT.fetch_add(1, Ordering::Relaxed);
        VALUES.with(|v| v.borrow_mut().insert(key, Rc::new(value) as Rc<dyn Any>));
        Local {
            key,
            _t: PhantomData,
        }
    }

    /// Runs `f` on the value; `None` when it was taken (or this is another thread).
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        // Clone the Rc out first: `f` may create or drop other `Local`s.
        let rc = VALUES.with(|v| v.borrow().get(&self.key).cloned())?;
        rc.downcast_ref::<T>().map(f)
    }

    /// Removes the value now (later `with` calls return `None`).
    pub fn take(&self) {
        let gone = VALUES.with(|v| v.borrow_mut().remove(&self.key));
        drop(gone);
    }
}

impl<T: 'static> Drop for Local<T> {
    fn drop(&mut self) {
        self.take();
    }
}

/// Runs `fut` (which may hold `JsValue`s) on the page's event loop now and returns a `Send`
/// future of its output. Dropping the returned future does not cancel the work.
pub fn on_page<T, F>(fut: F) -> impl Future<Output = Option<T>> + Send + 'static
where
    T: Send + 'static,
    F: Future<Output = T> + 'static,
{
    let (tx, rx) = oneshot::channel();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = tx.send(fut.await);
    });
    async move { rx.await.ok() }
}
