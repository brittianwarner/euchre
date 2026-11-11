//! The one async-runtime seam (web plan S1): every task, sleep and timeout in `net/` that also
//! builds for the browser goes through here, never through `tokio::` directly.
//!
//! Native runs on tokio (spawn, sleep, timeout). The browser has no tokio runtime and no
//! threads: tasks run on the page's event loop through `wasm_bindgen_futures::spawn_local`,
//! timers are `setTimeout`, and an aborted task is a `futures::future::Abortable`. Both sides
//! keep the same shapes (`Task::abort`, `Task::join`, `sleep`, `timeout`) and the same `Send`
//! bounds, so the bridge, the sim and the web backend compile unchanged against either.

use std::future::Future;
use std::time::Duration;

/// A timeout ran out before the future finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elapsed;

pub use imp::{Task, sleep, spawn};

/// Runs `fut` for at most `limit`.
pub async fn timeout<F: Future>(limit: Duration, fut: F) -> Result<F::Output, Elapsed> {
    imp::timeout(limit, fut).await
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::future::Future;
    use std::time::Duration;

    use super::Elapsed;

    /// A spawned task; `abort` cancels it.
    #[derive(Debug)]
    pub struct Task(tokio::task::JoinHandle<()>);

    impl Task {
        /// Cancels the task (a finished one is unaffected).
        pub fn abort(&self) {
            self.0.abort();
        }

        /// Waits for the task to end (it ran out, or it was aborted).
        pub async fn join(self) {
            let _ = self.0.await;
        }
    }

    /// Spawns `fut` on the current tokio runtime.
    pub fn spawn<F: Future<Output = ()> + Send + 'static>(fut: F) -> Task {
        Task(tokio::spawn(fut))
    }

    /// Sleeps for `d`.
    pub async fn sleep(d: Duration) {
        tokio::time::sleep(d).await;
    }

    pub(super) async fn timeout<F: Future>(limit: Duration, fut: F) -> Result<F::Output, Elapsed> {
        tokio::time::timeout(limit, fut).await.map_err(|_| Elapsed)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use futures::future::{AbortHandle, Abortable, Either, select};
    use parking_lot::Mutex;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    use super::Elapsed;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = setTimeout)]
        fn set_timeout(handler: &js_sys::Function, ms: i32) -> JsValue;
        #[wasm_bindgen(js_name = clearTimeout)]
        fn clear_timeout(id: &JsValue);
    }

    /// A spawned task; `abort` cancels it at its next poll.
    #[derive(Debug)]
    pub struct Task {
        abort: AbortHandle,
        done: Arc<Mutex<(bool, Option<Waker>)>>,
    }

    impl Task {
        /// Cancels the task (a finished one is unaffected).
        pub fn abort(&self) {
            self.abort.abort();
        }

        /// Waits for the task to end (it ran out, or it was aborted).
        pub async fn join(self) {
            let done = self.done;
            std::future::poll_fn(move |cx| {
                let mut d = done.lock();
                if d.0 {
                    Poll::Ready(())
                } else {
                    d.1 = Some(cx.waker().clone());
                    Poll::Pending
                }
            })
            .await;
        }
    }

    /// Runs `fut` on the page's event loop (the browser main thread).
    pub fn spawn<F: Future<Output = ()> + Send + 'static>(fut: F) -> Task {
        let (abort, registration) = AbortHandle::new_pair();
        let done: Arc<Mutex<(bool, Option<Waker>)>> = Arc::default();
        let finished = Arc::clone(&done);
        wasm_bindgen_futures::spawn_local(async move {
            let _ = Abortable::new(fut, registration).await;
            let waker = {
                let mut d = finished.lock();
                d.0 = true;
                d.1.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        });
        Task { abort, done }
    }

    #[derive(Default)]
    struct Shared {
        fired: bool,
        waker: Option<Waker>,
    }

    type Pending = (JsValue, Closure<dyn FnMut()>);

    thread_local! {
        /// Live `setTimeout`s by key. The callback is owned here, not by the future, so the
        /// future stays `Send` (wasm32 has one thread; the bound is only a formality there).
        static TIMERS: RefCell<HashMap<u64, Pending>> = RefCell::new(HashMap::new());
        static NEXT: Cell<u64> = const { Cell::new(1) };
    }

    /// A `setTimeout` as a future. Dropping it clears the timer.
    pub struct Sleep {
        ms: i32,
        shared: Arc<Mutex<Shared>>,
        key: Option<u64>,
    }

    /// Sleeps for `d` on the page's timer.
    pub fn sleep(d: Duration) -> Sleep {
        Sleep {
            ms: i32::try_from(d.as_millis()).unwrap_or(i32::MAX),
            shared: Arc::default(),
            key: None,
        }
    }

    impl Future for Sleep {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let this = self.get_mut();
            {
                let mut s = this.shared.lock();
                if s.fired {
                    return Poll::Ready(());
                }
                s.waker = Some(cx.waker().clone());
            }
            if this.key.is_none() {
                let shared = Arc::clone(&this.shared);
                let callback = Closure::<dyn FnMut()>::new(move || {
                    let waker = {
                        let mut s = shared.lock();
                        s.fired = true;
                        s.waker.take()
                    };
                    if let Some(w) = waker {
                        w.wake();
                    }
                });
                let id = set_timeout(callback.as_ref().unchecked_ref(), this.ms);
                let key = NEXT.with(|n| {
                    let k = n.get();
                    n.set(k + 1);
                    k
                });
                TIMERS.with(|t| t.borrow_mut().insert(key, (id, callback)));
                this.key = Some(key);
            }
            Poll::Pending
        }
    }

    impl Drop for Sleep {
        fn drop(&mut self) {
            if let Some(key) = self.key.take()
                && let Some((id, callback)) = TIMERS.with(|t| t.borrow_mut().remove(&key))
            {
                clear_timeout(&id);
                drop(callback);
            }
        }
    }

    pub(super) async fn timeout<F: Future>(limit: Duration, fut: F) -> Result<F::Output, Elapsed> {
        let fut = std::pin::pin!(fut);
        match select(fut, sleep(limit)).await {
            Either::Left((out, _)) => Ok(out),
            Either::Right(((), _)) => Err(Elapsed),
        }
    }
}
