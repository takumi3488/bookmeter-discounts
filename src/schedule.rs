//! Daily resident loop that replaces an external `CronJob` trigger.

use std::{future::Future, time::Duration};

use chrono::{DateTime, NaiveTime, Utc};
use tracing::{error, info};

/// Source of wall-clock time and sleeping, injectable for tests.
pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> {
        tokio::time::sleep(duration)
    }
}

/// Returns the first instant strictly after `now` whose UTC time of day is `at`.
#[must_use]
pub fn next_run_after(now: DateTime<Utc>, at: NaiveTime) -> DateTime<Utc> {
    let today = now.date_naive().and_time(at).and_utc();
    if today > now {
        today
    } else {
        today + chrono::Duration::days(1)
    }
}

/// Runs `job` once per day at `at` (UTC), serially, until `shutdown` resolves.
///
/// Startup and every finished run (success or failure) wait for the next slot
/// strictly in the future, so a slot missed by a long run or a restart is
/// skipped instead of caught up. `shutdown` cancels both the wait and an
/// in-flight run; no new run starts after it resolves.
pub async fn run_daily<C, F, Fut>(
    clock: &C,
    at: NaiveTime,
    shutdown: impl Future<Output = ()>,
    mut job: F,
) where
    C: Clock,
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let now = clock.now();
        let next = next_run_after(now, at);
        info!("next run at {next}");
        let wait = (next - now).to_std().unwrap_or_default();
        tokio::select! {
            () = clock.sleep(wait) => {}
            () = &mut shutdown => {
                info!("shutdown requested while waiting");
                return;
            }
        }
        info!("run started");
        tokio::select! {
            result = job() => match result {
                Ok(()) => info!("run finished"),
                Err(e) => error!("run failed: {e:?}"),
            },
            () = &mut shutdown => {
                info!("shutdown requested during run; aborting it");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap_or_default()
    }

    fn at() -> NaiveTime {
        NaiveTime::from_hms_opt(15, 2, 0).unwrap_or_default()
    }

    /// Fake clock: `sleep` advances time instantly; stops the test loop after `max_sleeps`.
    #[derive(Clone)]
    struct FakeClock {
        now: Rc<RefCell<DateTime<Utc>>>,
        sleeps: Rc<RefCell<usize>>,
        max_sleeps: usize,
    }

    impl FakeClock {
        fn new(start: &str, max_sleeps: usize) -> Self {
            Self {
                now: Rc::new(RefCell::new(t(start))),
                sleeps: Rc::new(RefCell::new(0)),
                max_sleeps,
            }
        }

        fn advance(&self, d: chrono::Duration) {
            let next = *self.now.borrow() + d;
            *self.now.borrow_mut() = next;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> DateTime<Utc> {
            *self.now.borrow()
        }

        async fn sleep(&self, duration: Duration) {
            *self.sleeps.borrow_mut() += 1;
            if *self.sleeps.borrow() > self.max_sleeps {
                std::future::pending::<()>().await;
            }
            self.advance(chrono::Duration::from_std(duration).unwrap_or_default());
        }
    }

    /// Resolves once the fake clock has slept `n` times past its limit, ending the loop.
    async fn after_limit(clock: FakeClock) {
        loop {
            if *clock.sleeps.borrow() > clock.max_sleeps {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn next_run_is_strictly_in_the_future() {
        assert_eq!(
            next_run_after(t("2026-11-25T07:00:00Z"), at()),
            t("2026-11-25T15:02:00Z")
        );
        assert_eq!(
            next_run_after(t("2026-11-25T15:02:00Z"), at()),
            t("2026-11-26T15:02:00Z")
        );
        assert_eq!(
            next_run_after(t("2026-11-25T16:00:00Z"), at()),
            t("2026-11-26T15:02:00Z")
        );
    }

    #[tokio::test]
    async fn runs_once_per_day_even_when_runs_fail() {
        let clock = FakeClock::new("2026-11-25T07:00:00Z", 3);
        let starts = Rc::new(RefCell::new(Vec::new()));
        let job_clock = clock.clone();
        let job_starts = Rc::clone(&starts);
        run_daily(&clock, at(), after_limit(clock.clone()), || {
            job_starts.borrow_mut().push(job_clock.now());
            job_clock.advance(chrono::Duration::hours(8));
            let fail = job_starts.borrow().len() == 1;
            async move {
                if fail {
                    anyhow::bail!("scrape failed")
                }
                Ok(())
            }
        })
        .await;
        assert_eq!(
            *starts.borrow(),
            vec![
                t("2026-11-25T15:02:00Z"),
                t("2026-11-26T15:02:00Z"),
                t("2026-11-27T15:02:00Z"),
            ]
        );
    }

    #[tokio::test]
    async fn overrunning_run_skips_missed_slot_instead_of_catching_up() {
        let clock = FakeClock::new("2026-11-25T07:00:00Z", 2);
        let starts = Rc::new(RefCell::new(Vec::new()));
        let job_clock = clock.clone();
        let job_starts = Rc::clone(&starts);
        run_daily(&clock, at(), after_limit(clock.clone()), || {
            job_starts.borrow_mut().push(job_clock.now());
            job_clock.advance(chrono::Duration::hours(25));
            async { Ok(()) }
        })
        .await;
        assert_eq!(
            *starts.borrow(),
            vec![t("2026-11-25T15:02:00Z"), t("2026-11-27T15:02:00Z")]
        );
    }

    #[tokio::test]
    async fn shutdown_while_waiting_starts_no_run() {
        let clock = FakeClock::new("2026-11-25T07:00:00Z", 0);
        let mut runs = 0;
        run_daily(&clock, at(), after_limit(clock.clone()), || {
            runs += 1;
            async { Ok(()) }
        })
        .await;
        assert_eq!(runs, 0);
    }

    #[tokio::test]
    async fn shutdown_during_run_aborts_it_and_starts_no_new_run() {
        let clock = FakeClock::new("2026-11-25T07:00:00Z", 1);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let mut tx = Some(tx);
        let mut runs = 0;
        let finished = Rc::new(RefCell::new(false));
        let job_finished = Rc::clone(&finished);
        run_daily(
            &clock,
            at(),
            async {
                let _ = rx.await;
            },
            || {
                runs += 1;
                if let Some(tx) = tx.take() {
                    let _ = tx.send(());
                }
                let job_finished = Rc::clone(&job_finished);
                async move {
                    std::future::pending::<()>().await;
                    *job_finished.borrow_mut() = true;
                    Ok(())
                }
            },
        )
        .await;
        assert_eq!(runs, 1);
        assert!(!*finished.borrow());
    }
}
