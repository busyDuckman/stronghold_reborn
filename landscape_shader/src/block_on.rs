// --------------------------------------------------------------------------------------------------------------------
// Run one future to completion on this thread. wgpu hands back futures for adapter and device
// requests and nothing else here is async, so a parking executor is all that is needed and
// saves pulling in a runtime for two calls at startup.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Drive the future, parking this thread between polls until it is ready.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker: Waker = Arc::new(ThreadWaker(thread::current())).into();
    let mut cx = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_futures_complete() {
        assert_eq!(block_on(async { 40 + 2 }), 42);
    }

    #[test]
    fn a_future_that_pends_once_completes() {
        struct PendOnce(bool);
        impl Future for PendOnce {
            type Output = &'static str;
            fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<&'static str> {
                if self.0 {
                    Poll::Ready("done")
                } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }
        assert_eq!(block_on(PendOnce(false)), "done");
    }
}
