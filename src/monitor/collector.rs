use std::collections::HashMap;

use sysinfo::{
    Disks, Networks, ProcessStatus as SysProcessStatus, ProcessesToUpdate, System, Users,
};

use crate::constants::*;
use crate::models::{
    BatteryInfo, BatteryStatus, CpuTemperature, DiskInfo, GpuInfo, NetworkInfo, ProcessInfo,
    ProcessStatus, SystemSnapshot,
};

/// Responsible for collecting system and process data.
/// Single Responsibility: only gathers data, no analysis.
pub struct SystemCollector {
    sys: System,
    users: Users,
    networks: Networks,
    disks: Disks,
    /// NVML handle (None if NVML not available)
    nvml: Option<nvml_wrapper::Nvml>,
    /// Previous disk I/O counters for delta calculation
    prev_disk_io: HashMap<String, (u64, u64)>,
    /// Timestamp of last collection for rate calculation
    last_collect: std::time::Instant,
}

impl SystemCollector {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        // Allow initial data to settle
        std::thread::sleep(std::time::Duration::from_millis(INITIAL_SETTLE_MS));
        sys.refresh_all();
        let users = Users::new_with_refreshed_list();
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();

        // Try to initialize NVML (will fail gracefully if no NVIDIA GPU)
        let nvml = nvml_wrapper::Nvml::init().ok();

        Self {
            sys,
            users,
            networks,
            disks,
            nvml,
            prev_disk_io: HashMap::new(),
            last_collect: std::time::Instant::now(),
        }
    }

    /// Refresh all system data and return fresh snapshots.
    pub fn collect(&mut self) -> (SystemSnapshot, Vec<ProcessInfo>) {
        self.sys.refresh_all();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.networks.refresh();
        self.disks.refresh();

        let system = self.collect_system();
        let processes = self.collect_processes();
        self.last_collect = std::time::Instant::now();
        (system, processes)
    }

    fn collect_system(&mut self) -> SystemSnapshot {
        let load_avg = System::load_average();

        // Collect per-interface network I/O
        let networks: Vec<NetworkInfo> = self
            .networks
            .iter()
            .map(|(name, data)| NetworkInfo {
                name: name.to_string(),
                rx_bytes: data.received(),
                tx_bytes: data.transmitted(),
                total_rx: data.total_received(),
                total_tx: data.total_transmitted(),
            })
            .collect();

        // Collect mounted filesystems (filter out tiny/virtual mounts < 1 GB)
        // Also compute disk I/O rates from /proc/diskstats
        let disk_io = read_disk_io_counters();
        let elapsed = self.last_collect.elapsed().as_secs_f64().max(0.1);

        let disks: Vec<DiskInfo> = self
            .disks
            .list()
            .iter()
            .filter(|d| d.total_space() >= MIN_DISK_SIZE_BYTES)
            .map(|d| {
                let name = d.name().to_string_lossy().to_string();
                // Strip /dev/ prefix for matching against /proc/diskstats
                let dev_name = name.strip_prefix("/dev/").unwrap_or(&name).to_string();

                let (read_rate, write_rate) =
                    if let Some(&(cur_read, cur_write)) = disk_io.get(&dev_name) {
                        if let Some(&(prev_read, prev_write)) = self.prev_disk_io.get(&dev_name) {
                            let dr = cur_read.saturating_sub(prev_read) as f64 / elapsed;
                            let dw = cur_write.saturating_sub(prev_write) as f64 / elapsed;
                            (dr as u64, dw as u64)
                        } else {
                            (0, 0)
                        }
                    } else {
                        (0, 0)
                    };

                DiskInfo {
                    mount_point: d.mount_point().to_string_lossy().to_string(),
                    fs_type: d.file_system().to_string_lossy().to_string(),
                    total_space: d.total_space(),
                    available_space: d.available_space(),
                    disk_kind: format!("{:?}", d.kind()),
                    read_bytes_per_sec: read_rate,
                    write_bytes_per_sec: write_rate,
                }
            })
            .collect();

        // Store current disk I/O counters for next delta
        self.prev_disk_io = disk_io;

        // Collect sensor data
        let cpu_temp = read_cpu_temperature();
        let gpu = self.read_gpu_info();
        let battery = read_battery_info();

        SystemSnapshot {
            total_memory: self.sys.total_memory(),
            used_memory: self.sys.used_memory(),
            total_swap: self.sys.total_swap(),
            used_swap: self.sys.used_swap(),
            cpu_count: self.sys.cpus().len(),
            cpu_usages: self.sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
            global_cpu_usage: self.sys.global_cpu_usage(),
            uptime: System::uptime(),
            hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
            os_name: format!(
                "{} {}",
                System::name().unwrap_or_else(|| "Unknown".into()),
                System::os_version().unwrap_or_else(|| String::new())
            ),
            load_avg_1: load_avg.one,
            load_avg_5: load_avg.five,
            load_avg_15: load_avg.fifteen,
            total_processes: self.sys.processes().len(),
            networks,
            disks,
            cpu_temp,
            gpu,
            battery,
        }
    }

    /// Read GPU info from NVML (NVIDIA only).
    fn read_gpu_info(&self) -> Option<GpuInfo> {
        let nvml = self.nvml.as_ref()?;
        let device = nvml.device_by_index(0).ok()?;

        let name = device.name().unwrap_or_else(|_| "NVIDIA GPU".to_string());
        let utilization = device.utilization_rates().map(|u| u.gpu).unwrap_or(0);
        let memory_info = device.memory_info().ok()?;
        let temperature = device
            .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
            .unwrap_or(0);
        let power_draw = device
            .power_usage()
            .map(|mw| mw as f32 / 1000.0) // milliwatts -> watts
            .unwrap_or(0.0);
        let fan_speed = device.fan_speed(0).ok();

        Some(GpuInfo {
            name,
            utilization,
            memory_used: memory_info.used,
            memory_total: memory_info.total,
            temperature,
            power_draw,
            fan_speed,
        })
    }

    fn collect_processes(&self) -> Vec<ProcessInfo> {
        self.sys
            .processes()
            .iter()
            .map(|(pid, proc_info)| {
                let cmd_parts: Vec<String> = proc_info
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect();
                let cmd = if cmd_parts.is_empty() {
                    proc_info.name().to_string_lossy().to_string()
                } else {
                    cmd_parts.join(" ")
                };

                ProcessInfo {
                    pid: pid.as_u32(),
                    name: proc_info.name().to_string_lossy().to_string(),
                    cmd,
                    cpu_usage: proc_info.cpu_usage(),
                    memory_bytes: proc_info.memory(),
                    memory_percent: if self.sys.total_memory() > 0 {
                        (proc_info.memory() as f32 / self.sys.total_memory() as f32) * 100.0
                    } else {
                        0.0
                    },
                    disk_read_bytes: proc_info.disk_usage().read_bytes,
                    disk_write_bytes: proc_info.disk_usage().written_bytes,
                    status: map_process_status(proc_info.status()),
                    user: proc_info
                        .user_id()
                        .and_then(|uid| self.users.get_user_by_id(uid))
                        .map(|u| u.name().to_string())
                        .unwrap_or_else(|| {
                            // Fallback to numeric UID if username not found
                            proc_info
                                .user_id()
                                .map(|u| u.to_string())
                                .unwrap_or_else(|| "-".into())
                        }),
                    start_time: proc_info.start_time(),
                    parent_pid: proc_info.parent().map(|p| p.as_u32()),
                    thread_count: proc_info.tasks().map(|t| t.len() as u32),
                }
            })
            .collect()
    }

    /// Get a reference to the underlying sysinfo System (for process signals).
    pub fn system(&self) -> &System {
        &self.sys
    }
}

fn map_process_status(status: SysProcessStatus) -> ProcessStatus {
    match status {
        SysProcessStatus::Run => ProcessStatus::Running,
        SysProcessStatus::Sleep | SysProcessStatus::Idle => ProcessStatus::Sleeping,
        SysProcessStatus::Stop => ProcessStatus::Stopped,
        SysProcessStatus::Zombie => ProcessStatus::Zombie,
        SysProcessStatus::Dead => ProcessStatus::Dead,
        _ => ProcessStatus::Unknown,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sensor reading functions (gracefully return None on failure)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Read CPU temperature from /sys/class/thermal and /sys/class/hwmon.
/// Returns None if no temperature sensors are available (e.g. WSL).
fn read_cpu_temperature() -> Option<CpuTemperature> {
    let mut package_temp: Option<f32> = None;
    let mut core_temps: Vec<f32> = Vec::new();

    // Try /sys/class/hwmon/hwmon*/temp*_input (more detailed, has per-core)
    if let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let path = entry.path();
            // Check if this hwmon is a CPU sensor (coretemp, k10temp, etc.)
            let name_path = path.join("name");
            let name = std::fs::read_to_string(&name_path)
                .unwrap_or_default()
                .trim()
                .to_string();

            if !matches!(
                name.as_str(),
                "coretemp" | "k10temp" | "zenpower" | "it8688" | "acpitz"
            ) {
                continue;
            }

            // Read temp*_input files
            for i in 1..=MAX_HWMON_SENSORS {
                let temp_path = path.join(format!("temp{}_input", i));
                if let Ok(val) = std::fs::read_to_string(&temp_path) {
                    if let Ok(millideg) = val.trim().parse::<f32>() {
                        let temp = millideg / 1000.0;
                        // Check label to distinguish package vs core
                        let label_path = path.join(format!("temp{}_label", i));
                        let label = std::fs::read_to_string(&label_path)
                            .unwrap_or_default()
                            .trim()
                            .to_lowercase();

                        if label.contains("package") || label.contains("tdie") || i == 1 {
                            package_temp = Some(temp);
                        } else if label.contains("core") {
                            core_temps.push(temp);
                        }
                    }
                }
            }

            if package_temp.is_some() || !core_temps.is_empty() {
                break; // Found CPU sensor, stop looking
            }
        }
    }

    // Fallback: try /sys/class/thermal/thermal_zone*
    if package_temp.is_none() && core_temps.is_empty() {
        for i in 0..MAX_THERMAL_ZONES {
            let temp_path = format!("/sys/class/thermal/thermal_zone{}/temp", i);
            let type_path = format!("/sys/class/thermal/thermal_zone{}/type", i);
            let zone_type = std::fs::read_to_string(&type_path)
                .unwrap_or_default()
                .trim()
                .to_lowercase();

            if zone_type.contains("cpu")
                || zone_type.contains("x86_pkg")
                || zone_type.contains("acpitz")
                || zone_type.contains("soc")
            {
                if let Ok(val) = std::fs::read_to_string(&temp_path) {
                    if let Ok(millideg) = val.trim().parse::<f32>() {
                        package_temp = Some(millideg / 1000.0);
                        break;
                    }
                }
            }
        }
    }

    if package_temp.is_some() || !core_temps.is_empty() {
        Some(CpuTemperature {
            package_temp,
            core_temps,
        })
    } else {
        None
    }
}

/// Read battery info from /sys/class/power_supply/.
/// Returns None if no battery is present (desktops, WSL).
fn read_battery_info() -> Option<BatteryInfo> {
    let ps_dir = std::fs::read_dir("/sys/class/power_supply").ok()?;

    for entry in ps_dir.flatten() {
        let path = entry.path();
        let ps_type = std::fs::read_to_string(path.join("type"))
            .unwrap_or_default()
            .trim()
            .to_string();

        if ps_type != "Battery" {
            continue;
        }

        // Read capacity (percentage)
        let percent = std::fs::read_to_string(path.join("capacity"))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(0.0);

        // Read status
        let status_str = std::fs::read_to_string(path.join("status"))
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let status = match status_str.as_str() {
            "charging" => BatteryStatus::Charging,
            "discharging" => BatteryStatus::Discharging,
            "full" => BatteryStatus::Full,
            "not charging" => BatteryStatus::NotCharging,
            _ => BatteryStatus::Unknown,
        };

        // Try to compute time remaining from energy/power
        let time_remaining = {
            let energy_now = std::fs::read_to_string(path.join("energy_now"))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());
            let power_now = std::fs::read_to_string(path.join("power_now"))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());
            let energy_full = std::fs::read_to_string(path.join("energy_full"))
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok());

            match (energy_now, power_now, energy_full) {
                (Some(now), Some(power), _)
                    if power > 0.0 && status == BatteryStatus::Discharging =>
                {
                    let hours = now / power;
                    let h = hours as u64;
                    let m = ((hours - h as f64) * 60.0) as u64;
                    Some(format!("{}h {:02}m", h, m))
                }
                (Some(now), Some(power), Some(full))
                    if power > 0.0 && status == BatteryStatus::Charging =>
                {
                    let hours = (full - now) / power;
                    let h = hours as u64;
                    let m = ((hours - h as f64) * 60.0) as u64;
                    Some(format!("{}h {:02}m to full", h, m))
                }
                _ => None,
            }
        };

        return Some(BatteryInfo {
            percent,
            status,
            time_remaining,
        });
    }

    None
}

/// Read disk I/O counters from /proc/diskstats.
/// Returns HashMap<device_name, (read_bytes, write_bytes)>.
fn read_disk_io_counters() -> HashMap<String, (u64, u64)> {
    let mut result = HashMap::new();

    let content = match std::fs::read_to_string("/proc/diskstats") {
        Ok(c) => c,
        Err(_) => return result,
    };

    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < MIN_DISKSTATS_FIELDS {
            continue;
        }

        let name = fields[2].to_string();
        // Skip ram, loop, dm- devices (partitions are fine)
        if name.starts_with("ram") || name.starts_with("loop") || name.starts_with("dm-") {
            continue;
        }

        // Field 5 = sectors read, Field 9 = sectors written
        // Sector size is typically 512 bytes
        let sectors_read: u64 = fields[5].parse().unwrap_or(0);
        let sectors_written: u64 = fields[9].parse().unwrap_or(0);
        let read_bytes = sectors_read * SECTOR_SIZE_BYTES;
        let write_bytes = sectors_written * SECTOR_SIZE_BYTES;

        result.insert(name, (read_bytes, write_bytes));
    }

    result
}
