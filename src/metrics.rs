//! In-memory runtime metrics.
//!
//! This module tracks counters/latencies for:
//! - tasks (runs)
//! - tool calls
//! - eval runs
//!
//! It’s intentionally simple (in-memory only) and is exposed over `GET /metrics`.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Default)]
struct LatencyAgg {
    count: AtomicU64,
    sum_ms: AtomicU64,
    max_ms: AtomicU64,
}

impl LatencyAgg {
    /// Record one latency measurement into the aggregate.
    fn record(&self, duration: Duration) {
        let ms = duration.as_millis().min(u128::from(u64::MAX)) as u64;
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_ms.fetch_add(ms, Ordering::Relaxed);
        let mut prev = self.max_ms.load(Ordering::Relaxed);
        while ms > prev {
            match self
                .max_ms
                .compare_exchange_weak(prev, ms, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(p) => prev = p,
            }
        }
    }

    /// Create a serializable snapshot (count/sum/avg/max).
    fn snapshot(&self) -> LatencyAggSnapshot {
        let count = self.count.load(Ordering::Relaxed);
        let sum_ms = self.sum_ms.load(Ordering::Relaxed);
        let max_ms = self.max_ms.load(Ordering::Relaxed);
        let avg_ms = if count == 0 {
            0.0
        } else {
            sum_ms as f64 / count as f64
        };
        LatencyAggSnapshot {
            count,
            sum_ms,
            avg_ms,
            max_ms,
        }
    }
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct LatencyAggSnapshot {
    pub count: u64,
    pub sum_ms: u64,
    pub avg_ms: f64,
    pub max_ms: u64,
}

#[derive(Debug)]
pub struct RuntimeMetrics {
    started_at: Instant,

    tasks_started: AtomicU64,
    tasks_completed: AtomicU64,
    tasks_failed: AtomicU64,
    task_latency: LatencyAgg,

    tool_calls_total: AtomicU64,
    tool_calls_success: AtomicU64,
    tool_calls_failed: AtomicU64,
    tool_latency: LatencyAgg,
    tool_by_name: Mutex<HashMap<String, ToolCountersAgg>>,

    eval_runs_total: AtomicU64,
    eval_runs_failed: AtomicU64,
    eval_latency: LatencyAgg,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ToolCounters {
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub latency: LatencyAggSnapshot,
}

#[derive(Debug, Default)]
struct ToolCountersAgg {
    total: u64,
    success: u64,
    failed: u64,
    latency: LatencyAgg,
}

impl ToolCountersAgg {
    /// Record one tool call into this per-tool aggregate.
    fn record(&mut self, ok: bool, duration: Duration) {
        self.total += 1;
        if ok {
            self.success += 1;
        } else {
            self.failed += 1;
        }
        self.latency.record(duration);
    }

    /// Create a serializable snapshot of per-tool counters/latency.
    fn snapshot(&self) -> ToolCounters {
        ToolCounters {
            total: self.total,
            success: self.success,
            failed: self.failed,
            latency: self.latency.snapshot(),
        }
    }
}

impl RuntimeMetrics {
    /// Create a fresh metrics collector (all counters zero).
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            tasks_started: AtomicU64::new(0),
            tasks_completed: AtomicU64::new(0),
            tasks_failed: AtomicU64::new(0),
            task_latency: LatencyAgg::default(),
            tool_calls_total: AtomicU64::new(0),
            tool_calls_success: AtomicU64::new(0),
            tool_calls_failed: AtomicU64::new(0),
            tool_latency: LatencyAgg::default(),
            tool_by_name: Mutex::new(HashMap::new()),
            eval_runs_total: AtomicU64::new(0),
            eval_runs_failed: AtomicU64::new(0),
            eval_latency: LatencyAgg::default(),
        }
    }

    /// Increment the “tasks started” counter.
    pub fn record_task_started(&self) {
        self.tasks_started.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a finished task (success/failure + latency).
    pub fn record_task_finished(&self, ok: bool, duration: Duration) {
        if ok {
            self.tasks_completed.fetch_add(1, Ordering::Relaxed);
        } else {
            self.tasks_failed.fetch_add(1, Ordering::Relaxed);
        }
        self.task_latency.record(duration);
    }

    /// Record a tool call (success/failure + latency), grouped by tool name.
    pub async fn record_tool_call(&self, tool: &str, ok: bool, duration: Duration) {
        self.tool_calls_total.fetch_add(1, Ordering::Relaxed);
        if ok {
            self.tool_calls_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.tool_calls_failed.fetch_add(1, Ordering::Relaxed);
        }
        self.tool_latency.record(duration);

        let mut by = self.tool_by_name.lock().await;
        let entry = by
            .entry(tool.to_string())
            .or_insert_with(ToolCountersAgg::default);
        entry.record(ok, duration);
    }

    /// Record a completed eval run (success/failure + latency).
    ///
    /// Connection:
    /// - Called by eval endpoints and deep health checks.
    /// - Exposed via `GET /metrics` for observability.
    pub fn record_eval_run(&self, ok: bool, duration: Duration) {
        self.eval_runs_total.fetch_add(1, Ordering::Relaxed);
        if !ok {
            self.eval_runs_failed.fetch_add(1, Ordering::Relaxed);
        }
        self.eval_latency.record(duration);
    }

    /// Build a point-in-time snapshot for the metrics endpoint.
    ///
    /// Connection:
    /// - `GET /metrics` calls this to return structured counters/latencies to the UI.
    pub async fn snapshot(&self) -> MetricsSnapshot {
        let tool_by_name = {
            let by = self.tool_by_name.lock().await;
            let mut out: HashMap<String, ToolCounters> = HashMap::new();
            for (k, v) in by.iter() {
                out.insert(k.clone(), v.snapshot());
            }
            out
        };

        MetricsSnapshot {
            uptime_ms: self.started_at.elapsed().as_millis() as u64,
            tasks: TaskMetricsSnapshot {
                started: self.tasks_started.load(Ordering::Relaxed),
                completed: self.tasks_completed.load(Ordering::Relaxed),
                failed: self.tasks_failed.load(Ordering::Relaxed),
                latency: self.task_latency.snapshot(),
            },
            tools: ToolMetricsSnapshot {
                total: self.tool_calls_total.load(Ordering::Relaxed),
                success: self.tool_calls_success.load(Ordering::Relaxed),
                failed: self.tool_calls_failed.load(Ordering::Relaxed),
                latency: self.tool_latency.snapshot(),
                by_name: tool_by_name,
            },
            eval: EvalMetricsSnapshot {
                runs: self.eval_runs_total.load(Ordering::Relaxed),
                failed: self.eval_runs_failed.load(Ordering::Relaxed),
                latency: self.eval_latency.snapshot(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub uptime_ms: u64,
    pub tasks: TaskMetricsSnapshot,
    pub tools: ToolMetricsSnapshot,
    pub eval: EvalMetricsSnapshot,
}

#[derive(Debug, Serialize)]
pub struct TaskMetricsSnapshot {
    pub started: u64,
    pub completed: u64,
    pub failed: u64,
    pub latency: LatencyAggSnapshot,
}

#[derive(Debug, Serialize)]
pub struct ToolMetricsSnapshot {
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub latency: LatencyAggSnapshot,
    pub by_name: HashMap<String, ToolCounters>,
}

#[derive(Debug, Serialize)]
pub struct EvalMetricsSnapshot {
    pub runs: u64,
    pub failed: u64,
    pub latency: LatencyAggSnapshot,
}

pub type SharedMetrics = Arc<RuntimeMetrics>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metrics_records_tool_calls() {
        let m = RuntimeMetrics::new();
        m.record_task_started();
        m.record_task_finished(true, Duration::from_millis(12));

        m.record_tool_call("shell", true, Duration::from_millis(5))
            .await;
        m.record_tool_call("shell", false, Duration::from_millis(7))
            .await;
        m.record_eval_run(true, Duration::from_millis(3));

        let snap = m.snapshot().await;
        assert_eq!(snap.tasks.started, 1);
        assert_eq!(snap.tasks.completed, 1);
        assert_eq!(snap.tools.total, 2);
        assert!(snap.tools.by_name.contains_key("shell"));
        assert_eq!(snap.eval.runs, 1);
    }
}
