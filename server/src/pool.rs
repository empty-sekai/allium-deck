//! Bounded pool of search threads.
//!
//! Deck search is CPU-bound and, for World Bloom and final-chapter requests, runs for
//! hundreds of milliseconds. Running that on the async runtime would starve the
//! connection handling, and handing it to an unbounded blocking pool would let load
//! spikes spawn arbitrarily many threads. So the runtime keeps a fixed number of
//! dedicated OS threads and a bounded queue in front of them:
//!
//! ```text
//! HTTP (async)  --try_send-->  sync_channel(max_queue)  -->  N search threads
//!                  full: 503                                  one job at a time
//! ```
//!
//! The queue is the backpressure signal. A full queue is answered immediately with 503
//! rather than absorbed, so a caller learns the service is saturated instead of waiting
//! behind an unbounded backlog.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

/// Work handed to a search thread. The closure reports its own result, which lets one
/// queue carry jobs with different return types.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// Why a request did not produce a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The queue was full. Retryable.
    QueueFull,
    /// The job was still queued when the wait budget ran out.
    QueueTimeout,
    /// The job panicked, or the pool is shutting down.
    Failed,
}

/// A finished job and how long it waited before a thread picked it up.
pub struct Completed<T> {
    pub value: T,
    pub queue_wait: Duration,
}

/// Counters exposed on `/metrics`.
#[derive(Debug, Default)]
pub struct PoolMetrics {
    pub accepted: AtomicU64,
    pub completed: AtomicU64,
    pub rejected_full: AtomicU64,
    pub rejected_timeout: AtomicU64,
    pub panics: AtomicU64,
    pub queued: AtomicU64,
    pub running: AtomicU64,
}

pub struct SearchPool {
    // `Option` so shutdown can drop the sender and let the threads observe the close.
    sender: Mutex<Option<mpsc::SyncSender<Job>>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    /// Budget for a job that has already started running, on top of the queue wait.
    execution_budget: Duration,
    queue_timeout: Duration,
    pub metrics: Arc<PoolMetrics>,
    pub workers_configured: usize,
    pub max_queue: usize,
}

impl SearchPool {
    /// Starts `workers` threads reading from a queue of at most `max_queue` jobs.
    ///
    /// `queue_timeout` bounds how long a job may sit unclaimed; `execution_budget`
    /// bounds how long it may run once claimed, and should exceed the largest search
    /// deadline the service will clamp to.
    pub fn new(
        workers: usize,
        max_queue: usize,
        queue_timeout: Duration,
        execution_budget: Duration,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Job>(max_queue);
        // One shared receiver: a thread holds the lock only while taking the next job,
        // so the threads hand it off in turn instead of contending for the whole run.
        let receiver = Arc::new(Mutex::new(receiver));
        let metrics = Arc::new(PoolMetrics::default());

        let handles = (0..workers)
            .map(|index| {
                let receiver = Arc::clone(&receiver);
                let metrics = Arc::clone(&metrics);
                std::thread::Builder::new()
                    .name(format!("deck-search-{index}"))
                    .spawn(move || worker_loop(&receiver, &metrics))
                    .map_err(|error| format!("spawning search thread {index} failed: {error}"))
            })
            .collect::<Result<Vec<_>, _>>();

        // A thread that cannot start is fatal at boot; there is no degraded mode that
        // would still honour the configured concurrency.
        let workers_started = match handles {
            Ok(handles) => handles,
            Err(error) => {
                tracing::error!("{error}");
                std::process::exit(1);
            }
        };

        Self {
            sender: Mutex::new(Some(sender)),
            workers: Mutex::new(workers_started),
            execution_budget,
            queue_timeout,
            metrics,
            workers_configured: workers,
            max_queue,
        }
    }

    /// Runs `task` on a search thread and awaits its result.
    ///
    /// Returns [`Rejection::QueueFull`] without waiting when the queue is saturated,
    /// and [`Rejection::QueueTimeout`] when the job was never claimed in time. Once a
    /// thread has started the job the wait extends to the execution budget, because the
    /// search has its own deadline and will finish on its own.
    pub async fn execute<T, F>(&self, task: F) -> Result<Completed<T>, Rejection>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (reply, mut receive) = oneshot::channel::<Completed<T>>();
        let started = Arc::new(AtomicBool::new(false));

        let job_started = Arc::clone(&started);
        let metrics = Arc::clone(&self.metrics);
        let enqueued_at = Instant::now();
        let job: Job = Box::new(move || {
            job_started.store(true, Ordering::Release);
            metrics.queued.fetch_sub(1, Ordering::Relaxed);
            metrics.running.fetch_add(1, Ordering::Relaxed);
            let queue_wait = enqueued_at.elapsed();
            let value = task();
            metrics.running.fetch_sub(1, Ordering::Relaxed);
            metrics.completed.fetch_add(1, Ordering::Relaxed);
            // A closed receiver means the caller gave up; dropping the value is right.
            let _ = reply.send(Completed { value, queue_wait });
        });

        let sender = {
            let guard = self.sender.lock().map_err(|_| Rejection::Failed)?;
            guard.as_ref().cloned().ok_or(Rejection::Failed)?
        };
        self.metrics.queued.fetch_add(1, Ordering::Relaxed);
        if sender.try_send(job).is_err() {
            self.metrics.queued.fetch_sub(1, Ordering::Relaxed);
            self.metrics.rejected_full.fetch_add(1, Ordering::Relaxed);
            return Err(Rejection::QueueFull);
        }
        self.metrics.accepted.fetch_add(1, Ordering::Relaxed);

        tokio::select! {
            result = &mut receive => result.map_err(|_| Rejection::Failed),
            () = tokio::time::sleep(self.queue_timeout) => {
                if !started.load(Ordering::Acquire) {
                    // The job is still queued and will still be picked up, so the depth
                    // gauge is left to the thread that consumes it. Only the caller
                    // stops waiting here.
                    self.metrics.rejected_timeout.fetch_add(1, Ordering::Relaxed);
                    return Err(Rejection::QueueTimeout);
                }
                // Already running: the search enforces its own deadline, so wait for it.
                match tokio::time::timeout(self.execution_budget, receive).await {
                    Ok(result) => result.map_err(|_| Rejection::Failed),
                    Err(_) => Err(Rejection::QueueTimeout),
                }
            }
        }
    }

    /// Stops accepting work and waits for the search threads to drain.
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.sender.lock() {
            guard.take();
        }
        if let Ok(mut guard) = self.workers.lock() {
            for handle in guard.drain(..) {
                let _ = handle.join();
            }
        }
    }
}

fn worker_loop(receiver: &Arc<Mutex<mpsc::Receiver<Job>>>, metrics: &Arc<PoolMetrics>) {
    loop {
        let job = {
            let Ok(guard) = receiver.lock() else {
                return;
            };
            match guard.recv() {
                Ok(job) => job,
                // Every sender is gone: the pool is shutting down.
                Err(_) => return,
            }
        };

        // A panicking job must not take the thread with it, or the configured
        // concurrency would silently shrink over the life of the process.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            metrics.panics.fetch_add(1, Ordering::Relaxed);
            metrics.running.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(workers: usize, max_queue: usize) -> SearchPool {
        SearchPool::new(
            workers,
            max_queue,
            Duration::from_millis(200),
            Duration::from_secs(5),
        )
    }

    #[tokio::test]
    async fn runs_a_job_and_reports_the_queue_wait() {
        let pool = pool(1, 4);
        let completed = pool.execute(|| 41 + 1).await;
        let Ok(completed) = completed else {
            panic!("job should have run");
        };
        assert_eq!(completed.value, 42);
        assert_eq!(pool.metrics.completed.load(Ordering::Relaxed), 1);
        pool.shutdown();
    }

    /// Waits for a pool state instead of assuming a scheduling order.
    ///
    /// Sleeping a fixed amount and hoping the spawned tasks ran in the order they were
    /// spawned makes these tests fail under load, so each step waits for the counter
    /// that proves the previous one landed.
    async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        for _ in 0..1_000 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn a_full_queue_is_rejected_rather_than_absorbed() {
        // One thread, queue of one: the third job has nowhere to go.
        let pool = Arc::new(pool(1, 1));
        let blocker = Arc::new(std::sync::Barrier::new(2));

        let held = Arc::clone(&blocker);
        let occupied = Arc::clone(&pool);
        let running = tokio::spawn(async move {
            occupied
                .execute(move || {
                    held.wait();
                })
                .await
                .is_ok()
        });
        let busy = Arc::clone(&pool);
        wait_until("the thread to be busy", move || {
            busy.metrics.running.load(Ordering::Relaxed) == 1
        })
        .await;

        // Fill the single queue slot.
        let queued = Arc::clone(&pool);
        let queued = tokio::spawn(async move { queued.execute(|| ()).await });
        let filled = Arc::clone(&pool);
        wait_until("the queue to fill", move || {
            filled.metrics.queued.load(Ordering::Relaxed) == 1
        })
        .await;

        // Now it has nowhere to go, and is refused rather than absorbed.
        let overflow = pool.execute(|| ()).await;
        assert_eq!(overflow.err(), Some(Rejection::QueueFull));
        assert_eq!(pool.metrics.rejected_full.load(Ordering::Relaxed), 1);

        blocker.wait();
        assert!(running.await.unwrap_or(false));
        let _ = queued.await;
        pool.shutdown();
    }

    #[tokio::test]
    async fn a_job_that_never_starts_times_out_in_the_queue() {
        let pool = Arc::new(pool(1, 4));
        let blocker = Arc::new(std::sync::Barrier::new(2));

        let held = Arc::clone(&blocker);
        let occupied = Arc::clone(&pool);
        let running =
            tokio::spawn(async move { occupied.execute(move || held.wait()).await.is_ok() });
        let busy = Arc::clone(&pool);
        wait_until("the thread to be busy", move || {
            busy.metrics.running.load(Ordering::Relaxed) == 1
        })
        .await;

        // The only thread is busy, so this one waits in the queue and gives up.
        let waiting = pool.execute(|| ()).await;
        assert_eq!(waiting.err(), Some(Rejection::QueueTimeout));
        assert_eq!(pool.metrics.rejected_timeout.load(Ordering::Relaxed), 1);

        blocker.wait();
        assert!(running.await.unwrap_or(false));

        // A timed-out caller stops waiting, but its job stays queued and still runs.
        // The depth gauge must therefore settle at zero rather than going negative.
        pool.shutdown();
        assert_eq!(pool.metrics.queued.load(Ordering::Relaxed), 0);
        assert_eq!(pool.metrics.completed.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn a_panicking_job_fails_one_request_and_keeps_the_thread() {
        let pool = pool(1, 4);
        let failed = pool.execute(|| panic!("boom")).await;
        assert_eq!(failed.err(), Some(Rejection::Failed));

        // The reply channel is dropped while the panic unwinds, so the caller learns the
        // job failed before the thread has caught the panic and counted it. Wait for the
        // counter rather than assuming the thread got there first.
        wait_until("the panic to be counted", || {
            pool.metrics.panics.load(Ordering::Relaxed) == 1
        })
        .await;

        // The same thread still serves the next request.
        let after = pool.execute(|| 7).await;
        assert_eq!(after.ok().map(|completed| completed.value), Some(7));
        pool.shutdown();
    }
}
