use bollard::container::{ListContainersOptions, Stats, StatsOptions};
use bollard::Docker;
use futures_util::StreamExt;

/// Information about a running Docker container.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub state: String,
    pub cpu_percent: f64,
    pub memory_usage: u64,
    pub memory_limit: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub pids: u64,
    pub created: i64,
}

impl ContainerInfo {
    pub fn memory_percent(&self) -> f64 {
        if self.memory_limit > 0 {
            (self.memory_usage as f64 / self.memory_limit as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// Docker monitor that connects via the local Unix socket.
pub struct DockerMonitor {
    client: Option<Docker>,
}

impl DockerMonitor {
    /// Try to connect to the Docker daemon. Returns a monitor even if Docker isn't available.
    pub fn new() -> Self {
        let client = Docker::connect_with_local_defaults().ok();
        Self { client }
    }

    /// Check if Docker is available.
    pub fn is_available(&self) -> bool {
        self.client.is_some()
    }

    /// List all containers (running and stopped) with their stats.
    pub async fn list_containers(&self) -> Vec<ContainerInfo> {
        let Some(client) = &self.client else {
            return Vec::new();
        };

        // List all containers
        let opts = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };

        let containers = match client.list_containers(Some(opts)).await {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut result = Vec::new();

        for container in containers {
            let id = container.id.clone().unwrap_or_default();
            let name = container
                .names
                .as_ref()
                .and_then(|n| n.first())
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| id[..12.min(id.len())].to_string());
            let image = container.image.clone().unwrap_or_default();
            let status = container.status.clone().unwrap_or_default();
            let state = container.state.clone().unwrap_or_default();
            let created = container.created.unwrap_or(0);

            // Only fetch stats for running containers
            let (cpu_percent, memory_usage, memory_limit, net_rx, net_tx, pids) =
                if state == "running" {
                    self.get_container_stats(client, &id).await
                } else {
                    (0.0, 0, 0, 0, 0, 0)
                };

            result.push(ContainerInfo {
                id: id[..12.min(id.len())].to_string(),
                name,
                image,
                status,
                state,
                cpu_percent,
                memory_usage,
                memory_limit,
                net_rx,
                net_tx,
                pids,
                created,
            });
        }

        result
    }

    async fn get_container_stats(
        &self,
        client: &Docker,
        id: &str,
    ) -> (f64, u64, u64, u64, u64, u64) {
        let opts = StatsOptions {
            stream: false,
            one_shot: true,
        };

        let mut stream = client.stats(id, Some(opts));

        if let Some(Ok(stats)) = stream.next().await {
            let cpu_percent = calculate_cpu_percent(&stats);
            let memory_usage = stats
                .memory_stats
                .usage
                .unwrap_or(0);
            let memory_limit = stats
                .memory_stats
                .limit
                .unwrap_or(0);

            // Network stats (sum all interfaces)
            let (net_rx, net_tx) = if let Some(ref networks) = stats.networks {
                networks.values().fold((0u64, 0u64), |(rx, tx), net| {
                    (rx + net.rx_bytes, tx + net.tx_bytes)
                })
            } else {
                (0, 0)
            };

            let pids = stats
                .pids_stats
                .current
                .unwrap_or(0);

            (cpu_percent, memory_usage, memory_limit, net_rx, net_tx, pids)
        } else {
            (0.0, 0, 0, 0, 0, 0)
        }
    }
}

/// Calculate CPU usage percentage from Docker stats.
fn calculate_cpu_percent(stats: &Stats) -> f64 {
    let cpu_delta = stats.cpu_stats.cpu_usage.total_usage as f64
        - stats.precpu_stats.cpu_usage.total_usage as f64;

    let system_delta = stats.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
        - stats.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;

    let num_cpus = stats
        .cpu_stats
        .online_cpus
        .unwrap_or(1) as f64;

    if system_delta > 0.0 && cpu_delta >= 0.0 {
        (cpu_delta / system_delta) * num_cpus * 100.0
    } else {
        0.0
    }
}
