//! `gauss-scheduler` — the background job engine.
//!
//! The reference platform uses Quartz for scheduled work (schema refresh,
//! alerts, subscriptions). GaussAnalytics replaces it with a small, dependency-
//! light Tokio scheduler: jobs implement the [`Job`] trait, are registered with
//! a fixed interval, and run on a [`Scheduler`]. The scheduling logic
//! ([`Scheduler::tick`]) is deterministic and unit-tested without real time;
//! [`Scheduler::run`] drives it on a Tokio timer with graceful shutdown.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use gauss_core::error::CoreResult;

/// A unit of recurring background work.
#[async_trait]
pub trait Job: Send + Sync {
    /// A stable name for logs/diagnostics.
    fn name(&self) -> &str;
    /// Execute one run of the job.
    async fn run(&self) -> CoreResult<()>;
}

/// A destination for alert/notification messages.
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, subject: &str, body: &str);
}

/// A notifier that records messages to the tracing log.
pub struct LogNotifier;

#[async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, subject: &str, body: &str) {
        tracing::info!(target: "gauss::alert", "{subject}: {body}");
    }
}

/// The JSON payload posted to a webhook. The `text` field is what Slack (and
/// most generic) incoming webhooks render.
pub fn webhook_payload(subject: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "text": format!("{subject}: {body}"),
        "subject": subject,
        "body": body,
    })
}

/// A notifier that POSTs alerts to a webhook URL (Slack incoming webhooks or any
/// generic JSON webhook). Failures are logged, not propagated.
pub struct WebhookNotifier {
    client: reqwest::Client,
    url: String,
}

impl WebhookNotifier {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.into(),
        }
    }
}

#[async_trait]
impl Notifier for WebhookNotifier {
    async fn notify(&self, subject: &str, body: &str) {
        let payload = webhook_payload(subject, body);
        if let Err(e) = self.client.post(&self.url).json(&payload).send().await {
            tracing::warn!(target: "gauss::alert", "webhook delivery failed: {e}");
        }
    }
}

struct Entry {
    name: String,
    interval: Duration,
    next_run: DateTime<Utc>,
    job: Arc<dyn Job>,
}

/// A registry of recurring jobs plus the loop that runs them.
#[derive(Default)]
pub struct Scheduler {
    entries: Vec<Entry>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `job` to run every `interval`, first due at `now + interval`.
    pub fn every(
        &mut self,
        name: impl Into<String>,
        interval: Duration,
        now: DateTime<Utc>,
        job: Arc<dyn Job>,
    ) {
        self.entries.push(Entry {
            name: name.into(),
            interval,
            next_run: now + interval,
            job,
        });
    }

    /// Number of registered jobs.
    pub fn job_count(&self) -> usize {
        self.entries.len()
    }

    /// Names of jobs due to run at `now`.
    pub fn due_names(&self, now: DateTime<Utc>) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.next_run <= now)
            .map(|e| e.name.clone())
            .collect()
    }

    /// Run all jobs due at `now`, rescheduling each to its next future slot.
    /// Returns each run's name and result.
    pub async fn tick(&mut self, now: DateTime<Utc>) -> Vec<(String, CoreResult<()>)> {
        let mut results = Vec::new();
        for e in &mut self.entries {
            if e.next_run <= now {
                let outcome = e.job.run().await;
                // Advance to the next slot strictly after `now` (skip missed slots).
                let mut next = e.next_run + e.interval;
                while next <= now {
                    next += e.interval;
                }
                e.next_run = next;
                results.push((e.name.clone(), outcome));
            }
        }
        results
    }

    /// Drive the scheduler on a wall-clock timer until `shutdown` flips to true.
    pub async fn run(
        mut self,
        period: StdDuration,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut ticker = tokio::time::interval(period);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    for (name, outcome) in self.tick(Utc::now()).await {
                        if let Err(e) = outcome {
                            tracing::warn!(target: "gauss::scheduler", "job {name} failed: {e}");
                        }
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingJob {
        name: String,
        runs: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Job for CountingJob {
        fn name(&self) -> &str {
            &self.name
        }
        async fn run(&self) -> CoreResult<()> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn webhook_payload_includes_text_for_slack() {
        let p = webhook_payload("Alert: too-many-errors", "5 rows matched");
        assert_eq!(
            p["text"].as_str().unwrap(),
            "Alert: too-many-errors: 5 rows matched"
        );
        assert_eq!(p["subject"].as_str().unwrap(), "Alert: too-many-errors");
    }

    #[tokio::test]
    async fn jobs_run_on_their_interval() {
        let t0 = Utc::now();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut sched = Scheduler::new();
        sched.every(
            "count",
            Duration::seconds(60),
            t0,
            Arc::new(CountingJob {
                name: "count".into(),
                runs: runs.clone(),
            }),
        );
        assert_eq!(sched.job_count(), 1);

        // Not due before the first interval elapses.
        assert!(sched.due_names(t0).is_empty());
        sched.tick(t0).await;
        assert_eq!(runs.load(Ordering::SeqCst), 0);

        // Due at t0 + 60s.
        let t1 = t0 + Duration::seconds(60);
        assert_eq!(sched.due_names(t1), vec!["count".to_string()]);
        sched.tick(t1).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        // Not due again immediately.
        sched.tick(t1 + Duration::seconds(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        // Due again at the next interval.
        sched.tick(t0 + Duration::seconds(120)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn missed_slots_are_skipped_not_replayed() {
        let t0 = Utc::now();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut sched = Scheduler::new();
        sched.every(
            "count",
            Duration::seconds(10),
            t0,
            Arc::new(CountingJob {
                name: "count".into(),
                runs: runs.clone(),
            }),
        );
        // Jump far ahead: the job runs once (catch-up), not once per missed slot.
        sched.tick(t0 + Duration::seconds(1000)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // And the next due time is in the future.
        assert!(sched.due_names(t0 + Duration::seconds(1000)).is_empty());
    }
}
