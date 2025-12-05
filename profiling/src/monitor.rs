//! Resource monitoring for stress testing.
//!
//! Provides background monitoring of system resources (CPU, memory)
//! and optional database metrics during stress tests.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use sysinfo::System;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// A single sample of system resource metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSample {
    /// Timestamp of the sample
    pub timestamp: DateTime<Utc>,
    /// CPU usage percentage (0.0 - 100.0)
    pub cpu_percent: f64,
    /// Resident set size (physical memory) in MB
    pub memory_rss_mb: f64,
    /// Virtual memory size in MB
    pub memory_vsz_mb: f64,
    /// Number of threads
    pub thread_count: u32,
}

/// Resource monitor that collects samples in the background.
pub struct ResourceMonitor {
    /// Interval between samples
    sampling_interval: Duration,
    /// Process ID to monitor (None = monitor system)
    pid: Option<u32>,
    /// Shared storage for samples
    samples: Arc<RwLock<Vec<ResourceSample>>>,
    /// Handle to the background task
    task_handle: Option<JoinHandle<()>>,
    /// Flag to stop monitoring
    stop_flag: Arc<tokio::sync::Notify>,
}

impl ResourceMonitor {
    /// Create a new resource monitor.
    ///
    /// If `pid` is None, monitors overall system resources.
    /// If `pid` is Some, monitors that specific process.
    pub fn new(sampling_interval: Duration, pid: Option<u32>) -> Self {
        Self {
            sampling_interval,
            pid,
            samples: Arc::new(RwLock::new(Vec::new())),
            task_handle: None,
            stop_flag: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Get the shared samples storage for external access.
    pub fn samples(&self) -> Arc<RwLock<Vec<ResourceSample>>> {
        self.samples.clone()
    }

    /// Start background sampling.
    pub fn start(&mut self) {
        let samples = self.samples.clone();
        let interval = self.sampling_interval;
        let pid = self.pid;
        let stop_flag = self.stop_flag.clone();

        let handle = tokio::spawn(async move {
            let mut sys = System::new_all();

            loop {
                // Check if we should stop
                tokio::select! {
                    _ = stop_flag.notified() => {
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {
                        // Continue with sampling
                    }
                }

                sys.refresh_all();

                let sample = if let Some(target_pid) = pid {
                    // Monitor specific process
                    let pid = sysinfo::Pid::from_u32(target_pid);
                    if let Some(process) = sys.process(pid) {
                        ResourceSample {
                            timestamp: Utc::now(),
                            cpu_percent: process.cpu_usage() as f64,
                            memory_rss_mb: process.memory() as f64 / 1024.0 / 1024.0,
                            memory_vsz_mb: process.virtual_memory() as f64 / 1024.0 / 1024.0,
                            thread_count: 0, // Not available per-process in sysinfo
                        }
                    } else {
                        continue; // Process not found
                    }
                } else {
                    // Monitor overall system
                    let cpu_usage = sys.global_cpu_usage() as f64;
                    let total_memory = sys.total_memory() as f64 / 1024.0 / 1024.0;
                    let used_memory = sys.used_memory() as f64 / 1024.0 / 1024.0;

                    ResourceSample {
                        timestamp: Utc::now(),
                        cpu_percent: cpu_usage,
                        memory_rss_mb: used_memory,
                        memory_vsz_mb: total_memory,
                        thread_count: 0,
                    }
                };

                let mut samples_guard = samples.write().await;
                samples_guard.push(sample);
            }
        });

        self.task_handle = Some(handle);
    }

    /// Stop background sampling and return collected samples.
    pub async fn stop(&mut self) -> Vec<ResourceSample> {
        self.stop_flag.notify_one();

        if let Some(handle) = self.task_handle.take() {
            let _ = handle.await;
        }

        let samples = self.samples.read().await;
        samples.clone()
    }

    /// Clear all collected samples.
    pub async fn clear(&self) {
        let mut samples = self.samples.write().await;
        samples.clear();
    }
}

/// Analysis of resource usage over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAnalysis {
    /// Number of samples analyzed
    pub sample_count: usize,
    /// Duration of sampling
    pub duration_secs: f64,
    /// Minimum CPU usage
    pub cpu_min: f64,
    /// Maximum CPU usage
    pub cpu_max: f64,
    /// Average CPU usage
    pub cpu_avg: f64,
    /// Minimum memory (RSS) in MB
    pub memory_min_mb: f64,
    /// Maximum memory (RSS) in MB
    pub memory_max_mb: f64,
    /// Average memory (RSS) in MB
    pub memory_avg_mb: f64,
    /// Memory growth rate (MB/sec) - positive indicates leak
    pub memory_growth_rate: f64,
    /// Whether a memory leak was detected
    pub memory_leak_detected: bool,
}

impl ResourceAnalysis {
    /// Analyze a set of resource samples.
    pub fn from_samples(samples: &[ResourceSample]) -> Self {
        if samples.is_empty() {
            return Self {
                sample_count: 0,
                duration_secs: 0.0,
                cpu_min: 0.0,
                cpu_max: 0.0,
                cpu_avg: 0.0,
                memory_min_mb: 0.0,
                memory_max_mb: 0.0,
                memory_avg_mb: 0.0,
                memory_growth_rate: 0.0,
                memory_leak_detected: false,
            };
        }

        let sample_count = samples.len();

        // Calculate duration
        let first_ts = samples.first().unwrap().timestamp;
        let last_ts = samples.last().unwrap().timestamp;
        let duration_secs = (last_ts - first_ts).num_milliseconds() as f64 / 1000.0;

        // CPU statistics
        let cpu_values: Vec<f64> = samples.iter().map(|s| s.cpu_percent).collect();
        let cpu_min = cpu_values.iter().cloned().fold(f64::INFINITY, f64::min);
        let cpu_max = cpu_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let cpu_avg = cpu_values.iter().sum::<f64>() / sample_count as f64;

        // Memory statistics
        let memory_values: Vec<f64> = samples.iter().map(|s| s.memory_rss_mb).collect();
        let memory_min_mb = memory_values.iter().cloned().fold(f64::INFINITY, f64::min);
        let memory_max_mb = memory_values
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let memory_avg_mb = memory_values.iter().sum::<f64>() / sample_count as f64;

        // Calculate memory growth rate using linear regression
        let memory_growth_rate = if duration_secs > 0.0 && sample_count > 1 {
            // Simple linear regression
            let n = sample_count as f64;
            let x_values: Vec<f64> = samples
                .iter()
                .map(|s| (s.timestamp - first_ts).num_milliseconds() as f64 / 1000.0)
                .collect();

            let x_sum: f64 = x_values.iter().sum();
            let y_sum: f64 = memory_values.iter().sum();
            let xy_sum: f64 = x_values
                .iter()
                .zip(memory_values.iter())
                .map(|(x, y)| x * y)
                .sum();
            let x2_sum: f64 = x_values.iter().map(|x| x * x).sum();

            let denominator = n * x2_sum - x_sum * x_sum;
            if denominator.abs() > 0.0001 {
                (n * xy_sum - x_sum * y_sum) / denominator
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Detect memory leak (growth rate > 0.1 MB/sec sustained)
        let memory_leak_detected = memory_growth_rate > 0.1 && duration_secs > 30.0;

        Self {
            sample_count,
            duration_secs,
            cpu_min,
            cpu_max,
            cpu_avg,
            memory_min_mb,
            memory_max_mb,
            memory_avg_mb,
            memory_growth_rate,
            memory_leak_detected,
        }
    }

    /// Generate a summary string.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str("Resource Analysis\n");
        s.push_str(&format!("  Samples:     {}\n", self.sample_count));
        s.push_str(&format!("  Duration:    {:.1}s\n", self.duration_secs));
        s.push_str(&format!(
            "  CPU:         {:.1}% min / {:.1}% avg / {:.1}% max\n",
            self.cpu_min, self.cpu_avg, self.cpu_max
        ));
        s.push_str(&format!(
            "  Memory:      {:.1} MB min / {:.1} MB avg / {:.1} MB max\n",
            self.memory_min_mb, self.memory_avg_mb, self.memory_max_mb
        ));
        s.push_str(&format!(
            "  Growth Rate: {:.3} MB/sec\n",
            self.memory_growth_rate
        ));
        if self.memory_leak_detected {
            s.push_str("  WARNING: Potential memory leak detected!\n");
        }
        s
    }
}

/// Database metrics for bottleneck detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseMetrics {
    /// Number of active connections
    pub active_connections: u32,
    /// Maximum connections allowed
    pub max_connections: u32,
    /// Number of connections waiting for a slot
    pub waiting_connections: u32,
    /// Transactions per second
    pub transactions_per_sec: f64,
    /// Cache hit ratio (0.0 - 1.0)
    pub cache_hit_ratio: f64,
    /// Number of dead tuples across tables
    pub dead_tuples: u64,
}

impl DatabaseMetrics {
    /// Check if connection pool is near exhaustion (>90% used).
    pub fn is_pool_exhausted(&self) -> bool {
        if self.max_connections == 0 {
            return false;
        }
        let utilization = self.active_connections as f64 / self.max_connections as f64;
        utilization > 0.9
    }

    /// Check if there are significant waiting connections.
    pub fn has_connection_contention(&self) -> bool {
        self.waiting_connections > 5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_analysis_empty() {
        let analysis = ResourceAnalysis::from_samples(&[]);
        assert_eq!(analysis.sample_count, 0);
        assert_eq!(analysis.duration_secs, 0.0);
    }

    #[test]
    fn test_resource_analysis_single_sample() {
        let samples = vec![ResourceSample {
            timestamp: Utc::now(),
            cpu_percent: 50.0,
            memory_rss_mb: 100.0,
            memory_vsz_mb: 200.0,
            thread_count: 10,
        }];
        let analysis = ResourceAnalysis::from_samples(&samples);
        assert_eq!(analysis.sample_count, 1);
        assert_eq!(analysis.cpu_avg, 50.0);
        assert_eq!(analysis.memory_avg_mb, 100.0);
    }

    #[test]
    fn test_memory_leak_detection() {
        // Simulate 60 seconds of samples with growing memory
        let start = Utc::now();
        let samples: Vec<ResourceSample> = (0..60)
            .map(|i| ResourceSample {
                timestamp: start + chrono::Duration::seconds(i),
                cpu_percent: 50.0,
                memory_rss_mb: 100.0 + (i as f64 * 0.5), // 0.5 MB/sec growth
                memory_vsz_mb: 200.0,
                thread_count: 10,
            })
            .collect();

        let analysis = ResourceAnalysis::from_samples(&samples);
        assert!(analysis.memory_growth_rate > 0.4);
        assert!(analysis.memory_leak_detected);
    }
}
