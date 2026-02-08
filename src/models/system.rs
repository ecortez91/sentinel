/// System-wide resource snapshot.
/// Provides the big-picture view of machine health.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SystemSnapshot {
    pub total_memory: u64,
    pub used_memory: u64,
    pub total_swap: u64,
    pub used_swap: u64,
    pub cpu_count: usize,
    pub cpu_usages: Vec<f32>,
    pub global_cpu_usage: f32,
    pub uptime: u64,
    pub hostname: String,
    pub os_name: String,
    pub load_avg_1: f64,
    pub load_avg_5: f64,
    pub load_avg_15: f64,
    pub total_processes: usize,
    /// Per-interface network I/O rates
    pub networks: Vec<NetworkInfo>,
    /// Mounted filesystem info
    pub disks: Vec<DiskInfo>,
    /// CPU temperature (may be None if sensors unavailable)
    pub cpu_temp: Option<CpuTemperature>,
    /// NVIDIA GPU info (may be None if no GPU or NVML unavailable)
    pub gpu: Option<GpuInfo>,
    /// Battery info (may be None if no battery / desktop)
    pub battery: Option<BatteryInfo>,
}

/// Network interface snapshot (rates since last refresh).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NetworkInfo {
    pub name: String,
    /// Bytes received since last refresh (delta)
    pub rx_bytes: u64,
    /// Bytes transmitted since last refresh (delta)
    pub tx_bytes: u64,
    /// Total bytes received since boot
    pub total_rx: u64,
    /// Total bytes transmitted since boot
    pub total_tx: u64,
}

/// Mounted filesystem snapshot.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DiskInfo {
    pub mount_point: String,
    pub fs_type: String,
    pub total_space: u64,
    pub available_space: u64,
    pub disk_kind: String,
    /// Disk I/O: read bytes/sec since last sample
    pub read_bytes_per_sec: u64,
    /// Disk I/O: write bytes/sec since last sample
    pub write_bytes_per_sec: u64,
}

/// CPU temperature readings.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CpuTemperature {
    /// Overall/package temperature in Celsius (if available)
    pub package_temp: Option<f32>,
    /// Per-core temperatures in Celsius (may be empty)
    pub core_temps: Vec<f32>,
}

/// NVIDIA GPU snapshot.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GpuInfo {
    pub name: String,
    pub utilization: u32,       // 0-100%
    pub memory_used: u64,       // bytes
    pub memory_total: u64,      // bytes
    pub temperature: u32,       // Celsius
    pub power_draw: f32,        // Watts
    pub fan_speed: Option<u32>, // 0-100%
}

impl GpuInfo {
    pub fn memory_percent(&self) -> f32 {
        if self.memory_total == 0 {
            return 0.0;
        }
        (self.memory_used as f32 / self.memory_total as f32) * 100.0
    }
}

/// Battery status.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BatteryInfo {
    pub percent: f32, // 0-100
    pub status: BatteryStatus,
    pub time_remaining: Option<String>, // e.g. "2h 15m"
}

#[derive(Debug, Clone, PartialEq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

impl SystemSnapshot {
    pub fn memory_percent(&self) -> f32 {
        if self.total_memory == 0 {
            return 0.0;
        }
        (self.used_memory as f32 / self.total_memory as f32) * 100.0
    }

    pub fn swap_percent(&self) -> f32 {
        if self.total_swap == 0 {
            return 0.0;
        }
        (self.used_swap as f32 / self.total_swap as f32) * 100.0
    }
}
