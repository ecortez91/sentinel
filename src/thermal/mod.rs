//! Thermal monitoring via LibreHardwareMonitor HTTP JSON API.
//!
//! LHM exposes a tree of hardware → sub-hardware → sensors at `/data.json`.
//! This module polls that endpoint, parses the JSON tree, and produces a
//! [`ThermalSnapshot`] with structured temperature, fan, and voltage data.
//!
//! Graceful fallback: if LHM is unreachable or the JSON format changes,
//! `poll()` returns `None` and the rest of Sentinel continues normally.

pub mod shutdown;

use std::time::Instant;

use serde::Deserialize;

/// A single sensor reading (temperature, fan RPM, etc.).
#[derive(Debug, Clone)]
pub struct SensorReading {
    /// Human-readable name (e.g. "CPU Core #1", "GPU Hot Spot").
    pub name: String,
    /// Parsed numeric value (°C for temps, RPM for fans).
    pub value: f32,
}

/// Complete thermal snapshot from one LHM poll.
#[derive(Debug, Clone)]
pub struct ThermalSnapshot {
    /// When this snapshot was captured.
    #[allow(dead_code)]
    pub timestamp: Instant,
    /// CPU package temperature (if available).
    pub cpu_package: Option<f32>,
    /// Per-core CPU temperatures.
    pub cpu_cores: Vec<SensorReading>,
    /// GPU temperature (if available).
    pub gpu_temp: Option<f32>,
    /// GPU hot spot temperature (if available).
    pub gpu_hotspot: Option<f32>,
    /// SSD / NVMe temperatures.
    pub ssd_temps: Vec<SensorReading>,
    /// Fan speeds in RPM.
    pub fan_rpms: Vec<SensorReading>,
    /// Motherboard / chipset temperatures.
    pub motherboard_temps: Vec<SensorReading>,
    /// Maximum temperature across all sensors (for alert checking).
    pub max_temp: f32,
    /// Maximum CPU temperature (package or highest core).
    pub max_cpu_temp: f32,
    /// Maximum GPU temperature (temp or hotspot).
    pub max_gpu_temp: f32,
}

impl ThermalSnapshot {
    /// Returns the highest temperature across all sensors.
    #[allow(dead_code)]
    pub fn overall_max(&self) -> f32 {
        self.max_temp
    }
}

/// Optional basic auth credentials for the LHM web server.
#[derive(Debug, Clone)]
pub struct LhmAuth {
    pub username: String,
    pub password: String,
}

impl LhmAuth {
    /// Load LHM auth from environment variables (if both are set).
    pub fn from_env() -> Option<Self> {
        let user = std::env::var(crate::constants::ENV_LHM_USER).ok()?;
        let pass = std::env::var(crate::constants::ENV_LHM_PASSWORD).ok()?;
        if user.is_empty() || pass.is_empty() {
            return None;
        }
        Some(Self {
            username: user,
            password: pass,
        })
    }
}

/// Client for polling LibreHardwareMonitor's HTTP JSON endpoint.
pub struct LhmClient {
    url: String,
    auth: Option<LhmAuth>,
    client: reqwest::Client,
}

impl LhmClient {
    /// Create a new client pointing at the given LHM URL with optional auth.
    pub fn new(url: &str, auth: Option<LhmAuth>) -> Self {
        Self {
            url: url.to_string(),
            auth,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Poll LHM and return a thermal snapshot, or None if unreachable / parse error.
    pub async fn poll(&self) -> Option<ThermalSnapshot> {
        let mut req = self.client.get(&self.url);
        if let Some(ref auth) = self.auth {
            req = req.basic_auth(&auth.username, Some(&auth.password));
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().await.ok()?;
        parse_lhm_json(&text)
    }
}

/// Detect the Windows host IP from WSL2 (reads /etc/resolv.conf nameserver).
/// Returns None if not running in WSL or detection fails.
pub fn detect_wsl_host_ip() -> Option<String> {
    let resolv = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    for line in resolv.lines() {
        let line = line.trim();
        if line.starts_with("nameserver") {
            let ip = line.split_whitespace().nth(1)?;
            // Sanity check: must look like an IP, not 127.x.x.x
            if !ip.starts_with("127.") && ip.contains('.') {
                return Some(ip.to_string());
            }
        }
    }
    None
}

/// Resolve the effective LHM URL, checking env override first, then config,
/// then auto-detecting WSL host IP if localhost doesn't work.
pub fn resolve_lhm_url(config_url: &str) -> String {
    // 1. Explicit env var override wins
    if let Ok(url) = std::env::var(crate::constants::ENV_LHM_URL) {
        if !url.is_empty() {
            return url;
        }
    }

    // 2. If config URL uses localhost and we're in WSL, substitute host IP
    if config_url.contains("localhost") || config_url.contains("127.0.0.1") {
        if let Some(host_ip) = detect_wsl_host_ip() {
            let resolved = config_url
                .replace("localhost", &host_ip)
                .replace("127.0.0.1", &host_ip);
            return resolved;
        }
    }

    // 3. Use config URL as-is
    config_url.to_string()
}

// ── LHM JSON structures ──────────────────────────────────────────

/// LHM tree node — represents a hardware item or sensor group.
/// The JSON is recursive: each node can have `Children` containing more nodes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct LhmNode {
    /// Display text, e.g. "Intel Core i7-10700K" or "CPU Core #1: 65 °C".
    #[serde(default)]
    text: String,
    /// Child nodes (sub-hardware, sensor categories, individual sensors).
    #[serde(default)]
    children: Vec<LhmNode>,
    /// Minimum value (present on sensor nodes).
    #[serde(default)]
    min: String,
    /// Maximum value (present on sensor nodes).
    #[serde(default)]
    max: String,
    /// Current value (present on sensor nodes), e.g. "65.2 °C" or "1200 RPM".
    #[serde(default)]
    value: String,
    /// Image URL path — used to identify node types:
    /// "images/transparent.png" for non-sensors,
    /// "images/cpu.png", "images/nvidia.png", etc. for hardware.
    #[serde(default, rename = "ImageURL")]
    image_url: String,
}

/// Parsed sensor with type info for classification.
#[derive(Debug)]
struct ParsedSensor {
    /// Full path breadcrumb (e.g. "Intel Core i7 > Temperatures > CPU Core #1").
    hardware_path: Vec<String>,
    /// Sensor name from the leaf node text (before the colon).
    name: String,
    /// Numeric value parsed from the text.
    value: f32,
    /// Category: "Temperatures", "Fans", "Clocks", etc.
    category: String,
}

/// Parse the LHM JSON tree into a ThermalSnapshot.
pub fn parse_lhm_json(json_str: &str) -> Option<ThermalSnapshot> {
    let root: LhmNode = serde_json::from_str(json_str).ok()?;

    // Flatten the tree into individual sensor readings
    let mut sensors = Vec::new();
    collect_sensors(&root, &mut Vec::new(), &mut String::new(), &mut sensors);

    if sensors.is_empty() {
        return None;
    }

    let mut snapshot = ThermalSnapshot {
        timestamp: Instant::now(),
        cpu_package: None,
        cpu_cores: Vec::new(),
        gpu_temp: None,
        gpu_hotspot: None,
        ssd_temps: Vec::new(),
        fan_rpms: Vec::new(),
        motherboard_temps: Vec::new(),
        max_temp: 0.0,
        max_cpu_temp: 0.0,
        max_gpu_temp: 0.0,
    };

    for sensor in &sensors {
        let name_lower = sensor.name.to_lowercase();
        let path_str = sensor.hardware_path.join(" ").to_lowercase();

        match sensor.category.as_str() {
            "Temperatures" => {
                if is_cpu_hardware(&path_str) {
                    // CPU temperatures — all contribute to max_temp
                    if sensor.value > snapshot.max_temp {
                        snapshot.max_temp = sensor.value;
                    }
                    if name_lower.contains("package") || name_lower.contains("cpu total") {
                        snapshot.cpu_package = Some(sensor.value);
                        if sensor.value > snapshot.max_cpu_temp {
                            snapshot.max_cpu_temp = sensor.value;
                        }
                    } else if name_lower.contains("core") || name_lower.contains("average") || name_lower.contains("max") {
                        snapshot.cpu_cores.push(SensorReading {
                            name: sensor.name.clone(),
                            value: sensor.value,
                        });
                        if sensor.value > snapshot.max_cpu_temp {
                            snapshot.max_cpu_temp = sensor.value;
                        }
                    } else {
                        // Other CPU temps (CCD, etc.)
                        if sensor.value > snapshot.max_cpu_temp {
                            snapshot.max_cpu_temp = sensor.value;
                        }
                    }
                } else if is_gpu_hardware(&path_str) {
                    // GPU temperatures — all contribute to max_temp
                    if sensor.value > snapshot.max_temp {
                        snapshot.max_temp = sensor.value;
                    }
                    if name_lower.contains("hot spot") || name_lower.contains("hotspot") {
                        snapshot.gpu_hotspot = Some(sensor.value);
                    } else if name_lower.contains("gpu") || name_lower.contains("temperature") {
                        snapshot.gpu_temp = Some(sensor.value);
                    }
                    if sensor.value > snapshot.max_gpu_temp {
                        snapshot.max_gpu_temp = sensor.value;
                    }
                } else if is_storage_hardware(&path_str) {
                    // Storage temps contribute to max_temp
                    if sensor.value > snapshot.max_temp {
                        snapshot.max_temp = sensor.value;
                    }
                    snapshot.ssd_temps.push(SensorReading {
                        name: format_storage_name(&sensor.hardware_path, &sensor.name),
                        value: sensor.value,
                    });
                } else {
                    // Motherboard / chipset / other
                    let is_mb_cpu = is_motherboard_cpu_sensor(&sensor.name);
                    snapshot.motherboard_temps.push(SensorReading {
                        name: if is_mb_cpu {
                            format!("{} (socket)", sensor.name)
                        } else {
                            sensor.name.clone()
                        },
                        value: sensor.value,
                    });
                    // Motherboard CPU socket sensor should NOT inflate max_temp —
                    // it's a less accurate proxy read by the Super I/O chip.
                    if !is_mb_cpu && sensor.value > snapshot.max_temp {
                        snapshot.max_temp = sensor.value;
                    }
                }
            }
            "Fans" => {
                snapshot.fan_rpms.push(SensorReading {
                    name: sensor.name.clone(),
                    value: sensor.value,
                });
            }
            _ => {}
        }
    }

    // Sort CPU cores by name for consistent display
    snapshot
        .cpu_cores
        .sort_by(|a, b| natural_sort_key(&a.name).cmp(&natural_sort_key(&b.name)));

    Some(snapshot)
}

// ── Tree walking ──────────────────────────────────────────────────

/// Recursively walk the LHM node tree, collecting leaf sensor nodes.
fn collect_sensors(
    node: &LhmNode,
    hardware_path: &mut Vec<String>,
    current_category: &mut String,
    out: &mut Vec<ParsedSensor>,
) {
    let text = node.text.trim().to_string();

    // Detect sensor category nodes (e.g. "Temperatures", "Fans", "Voltages")
    let is_category = matches!(
        text.as_str(),
        "Temperatures" | "Fans" | "Voltages" | "Clocks" | "Powers" | "Load" | "Data" | "Throughput"
    );

    if is_category {
        *current_category = text.clone();
    }

    // Check if this is a leaf sensor node (has a parseable value)
    if !node.value.is_empty() && node.children.is_empty() {
        if let Some(value) = parse_sensor_value(&node.value) {
            // Extract sensor name from text (before any colon)
            let name = if let Some(colon_pos) = text.find(':') {
                text[..colon_pos].trim().to_string()
            } else {
                text.clone()
            };

            // Skip noise/metadata sensors
            if is_noise_sensor(&name) {
                return;
            }

            if !current_category.is_empty() {
                out.push(ParsedSensor {
                    hardware_path: hardware_path.clone(),
                    name,
                    value,
                    category: current_category.clone(),
                });
            }
        }
    }

    // Recurse into children
    let is_hardware = !is_category && !text.is_empty() && node.value.is_empty();
    if is_hardware && !text.is_empty() {
        hardware_path.push(text);
    }

    for child in &node.children {
        collect_sensors(child, hardware_path, current_category, out);
    }

    if is_hardware {
        hardware_path.pop();
    }

    // Reset category when leaving a category branch
    if is_category {
        current_category.clear();
    }
}

/// Parse a sensor value string like "65.2 °C", "1200 RPM", "0.8 V" into f32.
fn parse_sensor_value(s: &str) -> Option<f32> {
    let s = s.trim();
    // Handle "no value" cases
    if s.is_empty() || s == "-" || s == "N/A" {
        return None;
    }
    // Extract leading number (possibly with decimal)
    let num_str: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == ',')
        .collect();
    let num_str = num_str.replace(',', ".");
    num_str.parse::<f32>().ok()
}

// ── Sensor filtering ─────────────────────────────────────────────

/// Returns true if a sensor name is metadata/noise that should be excluded.
/// These are LHM entries that look like sensors but are really config values
/// or diagnostic metadata from the chip driver (Nuvoton, ITE, etc.).
fn is_noise_sensor(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Distance to TjMax is an inverse metric, not an actual temperature
    if lower.contains("distance to tjmax") || lower.contains("tjmax") {
        return true;
    }
    // Chip diagnostic metadata, not real sensor readings
    if lower.contains("sensor resolution")
        || lower.contains("sensor low")
        || lower.contains("sensor high")
        || lower.contains("sensor limit")
    {
        return true;
    }
    // Threshold values reported as sensors
    if lower.starts_with("temperature warning")
        || lower.starts_with("temperature critical")
        || lower.starts_with("thermal sensor")
    {
        return true;
    }
    false
}

/// Returns true if this motherboard temperature sensor is actually reading the
/// CPU socket temperature (via the motherboard's Super I/O chip, e.g. Nuvoton).
/// This is a less accurate proxy for CPU die temp and should not inflate max_temp.
fn is_motherboard_cpu_sensor(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Exact match for "CPU" sensor on motherboard (Nuvoton, ITE chips)
    // but NOT "CPU Fan" or "CPU Opt" (those are fans, not temps)
    lower == "cpu"
        || lower == "cpu temperature"
        || lower == "cpu (peci)"
        || lower == "cpu peci"
}

// ── Hardware classification ──────────────────────────────────────

fn is_cpu_hardware(path: &str) -> bool {
    path.contains("cpu")
        || path.contains("intel core")
        || path.contains("amd ryzen")
        || path.contains("processor")
}

fn is_gpu_hardware(path: &str) -> bool {
    path.contains("gpu")
        || path.contains("nvidia")
        || path.contains("geforce")
        || path.contains("radeon")
        || path.contains("amd rx")
        || path.contains("intel arc")
}

fn is_storage_hardware(path: &str) -> bool {
    path.contains("ssd")
        || path.contains("nvme")
        || path.contains("samsung")
        || path.contains("wd ")
        || path.contains("western digital")
        || path.contains("crucial")
        || path.contains("kingston")
        || path.contains("hynix")
}

/// Format a storage sensor name to include the drive identifier.
fn format_storage_name(path: &[String], sensor_name: &str) -> String {
    // Find the storage device name in the path
    if let Some(dev) = path.last() {
        if dev.to_lowercase() != "temperatures" {
            return format!("{}: {}", dev, sensor_name);
        }
    }
    sensor_name.to_string()
}

/// Natural sort key: extract numeric suffix for sorting "Core #1", "Core #2", etc.
fn natural_sort_key(s: &str) -> (String, u32) {
    // Split at last sequence of digits
    if let Some(pos) = s.rfind(|c: char| c.is_ascii_digit()) {
        let start = s[..=pos]
            .rfind(|c: char| !c.is_ascii_digit())
            .map(|p| p + 1)
            .unwrap_or(0);
        let prefix = s[..start].to_string();
        let num = s[start..=pos].parse::<u32>().unwrap_or(0);
        (prefix, num)
    } else {
        (s.to_string(), 0)
    }
}

// ── Display formatting ───────────────────────────────────────────

impl ThermalSnapshot {
    /// Format the snapshot as a readable text block for the :thermal command.
    pub fn to_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push("=== Thermal Snapshot ===".to_string());
        lines.push(String::new());

        // CPU
        if self.cpu_package.is_some() || !self.cpu_cores.is_empty() {
            lines.push("CPU:".to_string());
            if let Some(pkg) = self.cpu_package {
                lines.push(format!("  Package: {:.1}°C", pkg));
            }
            for core in &self.cpu_cores {
                lines.push(format!("  {}: {:.1}°C", core.name, core.value));
            }
            lines.push(String::new());
        }

        // GPU
        if self.gpu_temp.is_some() || self.gpu_hotspot.is_some() {
            lines.push("GPU:".to_string());
            if let Some(t) = self.gpu_temp {
                lines.push(format!("  Temperature: {:.1}°C", t));
            }
            if let Some(t) = self.gpu_hotspot {
                lines.push(format!("  Hot Spot: {:.1}°C", t));
            }
            lines.push(String::new());
        }

        // Storage
        if !self.ssd_temps.is_empty() {
            lines.push("Storage:".to_string());
            for s in &self.ssd_temps {
                lines.push(format!("  {}: {:.1}°C", s.name, s.value));
            }
            lines.push(String::new());
        }

        // Motherboard
        if !self.motherboard_temps.is_empty() {
            lines.push("Motherboard:".to_string());
            for s in &self.motherboard_temps {
                lines.push(format!("  {}: {:.1}°C", s.name, s.value));
            }
            lines.push(String::new());
        }

        // Fans
        if !self.fan_rpms.is_empty() {
            lines.push("Fans:".to_string());
            for s in &self.fan_rpms {
                lines.push(format!("  {}: {:.0} RPM", s.name, s.value));
            }
            lines.push(String::new());
        }

        lines.push(format!("Max CPU: {:.1}°C", self.max_cpu_temp));
        lines.push(format!("Max GPU: {:.1}°C", self.max_gpu_temp));
        lines.push(format!("Overall Max: {:.1}°C", self.max_temp));

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sensor_value_celsius() {
        assert!((parse_sensor_value("65.2 °C").unwrap() - 65.2).abs() < 0.01);
    }

    #[test]
    fn parse_sensor_value_rpm() {
        assert!((parse_sensor_value("1200 RPM").unwrap() - 1200.0).abs() < 0.01);
    }

    #[test]
    fn parse_sensor_value_empty() {
        assert!(parse_sensor_value("").is_none());
        assert!(parse_sensor_value("-").is_none());
        assert!(parse_sensor_value("N/A").is_none());
    }

    #[test]
    fn parse_sensor_value_comma_decimal() {
        assert!((parse_sensor_value("65,3 °C").unwrap() - 65.3).abs() < 0.01);
    }

    #[test]
    fn natural_sort_key_cores() {
        let mut names = vec!["Core #10", "Core #2", "Core #1", "Core #9"];
        names.sort_by(|a, b| natural_sort_key(a).cmp(&natural_sort_key(b)));
        assert_eq!(names, vec!["Core #1", "Core #2", "Core #9", "Core #10"]);
    }

    #[test]
    fn natural_sort_key_no_number() {
        let key = natural_sort_key("Package");
        assert_eq!(key, ("Package".to_string(), 0));
    }

    /// Minimal LHM JSON tree for testing the parser.
    fn sample_lhm_json() -> &'static str {
        r#"{
            "id": 0,
            "Text": "Sensor",
            "Min": "",
            "Max": "",
            "Value": "",
            "ImageURL": "",
            "Children": [
                {
                    "id": 1,
                    "Text": "Intel Core i7-10700K",
                    "Min": "",
                    "Max": "",
                    "Value": "",
                    "ImageURL": "images/cpu.png",
                    "Children": [
                        {
                            "id": 2,
                            "Text": "Temperatures",
                            "Min": "",
                            "Max": "",
                            "Value": "",
                            "ImageURL": "",
                            "Children": [
                                {
                                    "id": 3,
                                    "Text": "CPU Package: 72.0 °C",
                                    "Min": "35.0 °C",
                                    "Max": "85.0 °C",
                                    "Value": "72.0 °C",
                                    "ImageURL": "",
                                    "Children": []
                                },
                                {
                                    "id": 4,
                                    "Text": "CPU Core #1: 70.0 °C",
                                    "Min": "33.0 °C",
                                    "Max": "83.0 °C",
                                    "Value": "70.0 °C",
                                    "ImageURL": "",
                                    "Children": []
                                },
                                {
                                    "id": 5,
                                    "Text": "CPU Core #2: 68.5 °C",
                                    "Min": "32.0 °C",
                                    "Max": "82.0 °C",
                                    "Value": "68.5 °C",
                                    "ImageURL": "",
                                    "Children": []
                                }
                            ]
                        }
                    ]
                },
                {
                    "id": 10,
                    "Text": "NVIDIA GeForce RTX 3080",
                    "Min": "",
                    "Max": "",
                    "Value": "",
                    "ImageURL": "images/nvidia.png",
                    "Children": [
                        {
                            "id": 11,
                            "Text": "Temperatures",
                            "Min": "",
                            "Max": "",
                            "Value": "",
                            "ImageURL": "",
                            "Children": [
                                {
                                    "id": 12,
                                    "Text": "GPU Core: 65.0 °C",
                                    "Min": "30.0 °C",
                                    "Max": "75.0 °C",
                                    "Value": "65.0 °C",
                                    "ImageURL": "",
                                    "Children": []
                                },
                                {
                                    "id": 13,
                                    "Text": "GPU Hot Spot: 78.0 °C",
                                    "Min": "35.0 °C",
                                    "Max": "88.0 °C",
                                    "Value": "78.0 °C",
                                    "ImageURL": "",
                                    "Children": []
                                }
                            ]
                        },
                        {
                            "id": 14,
                            "Text": "Fans",
                            "Min": "",
                            "Max": "",
                            "Value": "",
                            "ImageURL": "",
                            "Children": [
                                {
                                    "id": 15,
                                    "Text": "GPU Fan: 1500 RPM",
                                    "Min": "0 RPM",
                                    "Max": "2200 RPM",
                                    "Value": "1500 RPM",
                                    "ImageURL": "",
                                    "Children": []
                                }
                            ]
                        }
                    ]
                },
                {
                    "id": 20,
                    "Text": "Samsung SSD 970 EVO Plus",
                    "Min": "",
                    "Max": "",
                    "Value": "",
                    "ImageURL": "images/hdd.png",
                    "Children": [
                        {
                            "id": 21,
                            "Text": "Temperatures",
                            "Min": "",
                            "Max": "",
                            "Value": "",
                            "ImageURL": "",
                            "Children": [
                                {
                                    "id": 22,
                                    "Text": "Temperature: 42.0 °C",
                                    "Min": "25.0 °C",
                                    "Max": "55.0 °C",
                                    "Value": "42.0 °C",
                                    "ImageURL": "",
                                    "Children": []
                                }
                            ]
                        }
                    ]
                }
            ]
        }"#
    }

    #[test]
    fn parse_lhm_json_full_tree() {
        let snapshot = parse_lhm_json(sample_lhm_json()).expect("Should parse sample JSON");

        // CPU
        assert!((snapshot.cpu_package.unwrap() - 72.0).abs() < 0.01);
        assert_eq!(snapshot.cpu_cores.len(), 2);
        assert!((snapshot.cpu_cores[0].value - 70.0).abs() < 0.01);
        assert!((snapshot.cpu_cores[1].value - 68.5).abs() < 0.01);
        assert!(snapshot.cpu_cores[0].name.contains("Core #1"));
        assert!(snapshot.cpu_cores[1].name.contains("Core #2"));

        // GPU
        assert!((snapshot.gpu_temp.unwrap() - 65.0).abs() < 0.01);
        assert!((snapshot.gpu_hotspot.unwrap() - 78.0).abs() < 0.01);

        // Fans
        assert_eq!(snapshot.fan_rpms.len(), 1);
        assert!((snapshot.fan_rpms[0].value - 1500.0).abs() < 0.01);

        // Storage
        assert_eq!(snapshot.ssd_temps.len(), 1);
        assert!((snapshot.ssd_temps[0].value - 42.0).abs() < 0.01);

        // Max temps
        assert!((snapshot.max_cpu_temp - 72.0).abs() < 0.01);
        assert!((snapshot.max_gpu_temp - 78.0).abs() < 0.01);
        assert!((snapshot.max_temp - 78.0).abs() < 0.01); // GPU hot spot is highest
    }

    #[test]
    fn parse_lhm_json_empty_returns_none() {
        assert!(parse_lhm_json("{}").is_none());
        assert!(parse_lhm_json("not json").is_none());
    }

    #[test]
    fn thermal_snapshot_to_text() {
        let snapshot = parse_lhm_json(sample_lhm_json()).expect("Should parse");
        let text = snapshot.to_text();
        assert!(text.contains("CPU:"));
        assert!(text.contains("Package: 72.0°C"));
        assert!(text.contains("GPU:"));
        assert!(text.contains("Hot Spot: 78.0°C"));
        assert!(text.contains("Fans:"));
        assert!(text.contains("1500 RPM"));
        assert!(text.contains("Storage:"));
    }

    #[test]
    fn is_cpu_hardware_detects_intel_amd() {
        assert!(is_cpu_hardware("intel core i7-10700k"));
        assert!(is_cpu_hardware("amd ryzen 9 5900x"));
        assert!(is_cpu_hardware("some cpu thing"));
        assert!(!is_gpu_hardware("intel core i7-10700k"));
    }

    #[test]
    fn is_gpu_hardware_detects_nvidia_amd() {
        assert!(is_gpu_hardware("nvidia geforce rtx 3080"));
        assert!(is_gpu_hardware("amd radeon rx 6800"));
        assert!(!is_cpu_hardware("nvidia geforce rtx 3080"));
    }

    #[test]
    fn is_storage_hardware_detects_drives() {
        assert!(is_storage_hardware("samsung ssd 970 evo plus"));
        assert!(is_storage_hardware("nvme drive"));
        assert!(is_storage_hardware("wd black sn750"));
    }

    #[test]
    fn detect_wsl_host_ip_returns_some_on_wsl() {
        // This test verifies the helper works on WSL (returns Some)
        // or gracefully returns None on non-WSL systems.
        let ip = detect_wsl_host_ip();
        if std::path::Path::new("/etc/resolv.conf").exists() {
            // On WSL2 the nameserver should be the host IP
            // On native Linux it may be 127.0.0.53 which we skip
            if let Some(ref addr) = ip {
                assert!(addr.contains('.'), "Should be an IPv4 address");
                assert!(!addr.starts_with("127."), "Should not be loopback");
            }
        }
        // Either way, no panic — the function is safe
    }

    #[test]
    fn resolve_lhm_url_substitutes_wsl_host() {
        let resolved = resolve_lhm_url("http://localhost:8085/data.json");
        // On WSL, localhost should be replaced with the host IP
        // On non-WSL, it stays as localhost
        if detect_wsl_host_ip().is_some() {
            assert!(
                !resolved.contains("localhost"),
                "Should replace localhost on WSL, got: {}",
                resolved
            );
            assert!(resolved.contains(":8085/data.json"));
        } else {
            assert_eq!(resolved, "http://localhost:8085/data.json");
        }
    }

    #[test]
    fn resolve_lhm_url_respects_env_override() {
        // Temporarily set the env var
        let key = crate::constants::ENV_LHM_URL;
        let original = std::env::var(key).ok();
        std::env::set_var(key, "http://custom:9999/data.json");
        let resolved = resolve_lhm_url("http://localhost:8085/data.json");
        assert_eq!(resolved, "http://custom:9999/data.json");
        // Restore
        match original {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn lhm_auth_from_env_returns_none_when_missing() {
        // Clear env vars to ensure clean state
        let user_key = crate::constants::ENV_LHM_USER;
        let pass_key = crate::constants::ENV_LHM_PASSWORD;
        let orig_user = std::env::var(user_key).ok();
        let orig_pass = std::env::var(pass_key).ok();
        std::env::remove_var(user_key);
        std::env::remove_var(pass_key);
        assert!(LhmAuth::from_env().is_none());
        // Restore
        if let Some(v) = orig_user { std::env::set_var(user_key, v); }
        if let Some(v) = orig_pass { std::env::set_var(pass_key, v); }
    }

    #[test]
    fn lhm_auth_from_env_returns_some_when_set() {
        let user_key = crate::constants::ENV_LHM_USER;
        let pass_key = crate::constants::ENV_LHM_PASSWORD;
        let orig_user = std::env::var(user_key).ok();
        let orig_pass = std::env::var(pass_key).ok();
        std::env::set_var(user_key, "testuser");
        std::env::set_var(pass_key, "testpass");
        let auth = LhmAuth::from_env().expect("Should load auth from env");
        assert_eq!(auth.username, "testuser");
        assert_eq!(auth.password, "testpass");
        // Restore
        match orig_user {
            Some(v) => std::env::set_var(user_key, v),
            None => std::env::remove_var(user_key),
        }
        match orig_pass {
            Some(v) => std::env::set_var(pass_key, v),
            None => std::env::remove_var(pass_key),
        }
    }

    /// Integration test: poll the real LHM server running on the Windows host.
    ///
    /// This test loads .env credentials, resolves the WSL host IP, and
    /// fetches a live thermal snapshot. It is skipped if LHM is not reachable.
    #[tokio::test]
    async fn live_lhm_poll_returns_thermal_data() {
        // Load .env from the sentinel config directory
        let env_path = crate::constants::env_file_path();
        let _ = dotenvy::from_path(&env_path);

        // Resolve URL (auto-detects WSL host IP)
        let url = resolve_lhm_url(crate::constants::DEFAULT_LHM_URL);
        let auth = LhmAuth::from_env();

        // If no auth configured, skip (CI or no .env)
        if auth.is_none() {
            eprintln!("SKIP: No LHM auth configured (no .env with SENTINEL_LHM_USER/PASSWORD)");
            return;
        }

        let client = LhmClient::new(&url, auth);
        let snapshot = client.poll().await;

        match snapshot {
            Some(snap) => {
                // We got real data from LHM!
                eprintln!("=== LIVE THERMAL DATA ===");
                eprintln!("{}", snap.to_text());

                // Basic sanity: max_temp should be a reasonable value (> 0, < 150)
                assert!(snap.max_temp > 0.0, "Max temp should be > 0, got {}", snap.max_temp);
                assert!(snap.max_temp < 150.0, "Max temp should be < 150, got {}", snap.max_temp);

                // We should have at least some sensors
                let total_sensors = snap.cpu_cores.len()
                    + snap.cpu_package.is_some() as usize
                    + snap.gpu_temp.is_some() as usize
                    + snap.gpu_hotspot.is_some() as usize
                    + snap.ssd_temps.len()
                    + snap.fan_rpms.len()
                    + snap.motherboard_temps.len();
                assert!(total_sensors > 0, "Should have at least one sensor reading");

                eprintln!("Total sensors: {}", total_sensors);
                eprintln!("Max temp: {:.1}°C", snap.max_temp);
                eprintln!("Max CPU: {:.1}°C", snap.max_cpu_temp);
                eprintln!("Max GPU: {:.1}°C", snap.max_gpu_temp);
            }
            None => {
                eprintln!(
                    "SKIP: LHM not reachable at {} (server may be offline)",
                    url
                );
            }
        }
    }

    #[test]
    fn is_noise_sensor_filters_tjmax() {
        assert!(is_noise_sensor("Core #1 Distance to TjMax"));
        assert!(is_noise_sensor("CPU Core #3 (TjMax)"));
        assert!(!is_noise_sensor("CPU Core #1"));
        assert!(!is_noise_sensor("CPU Package"));
    }

    #[test]
    fn is_noise_sensor_filters_metadata() {
        assert!(is_noise_sensor("Temperature Sensor Resolution"));
        assert!(is_noise_sensor("Thermal Sensor Low Limit"));
        assert!(is_noise_sensor("Thermal Sensor High Limit"));
        assert!(is_noise_sensor("Thermal Sensor Critical Limit"));
        assert!(is_noise_sensor("Temperature warning"));
        assert!(is_noise_sensor("Temperature critical"));
        assert!(!is_noise_sensor("System"));
        assert!(!is_noise_sensor("PCH"));
    }

    #[test]
    fn is_motherboard_cpu_sensor_detects_socket() {
        assert!(is_motherboard_cpu_sensor("CPU"));
        assert!(is_motherboard_cpu_sensor("CPU Temperature"));
        assert!(is_motherboard_cpu_sensor("CPU (PECI)"));
        assert!(!is_motherboard_cpu_sensor("CPU Fan"));
        assert!(!is_motherboard_cpu_sensor("System"));
        assert!(!is_motherboard_cpu_sensor("PCH"));
    }

    #[test]
    fn noise_sensors_excluded_from_parse() {
        // Build a JSON tree with noise sensors alongside real ones
        let json = r#"{
            "id": 0, "Text": "Sensor", "Min": "", "Max": "", "Value": "", "ImageURL": "",
            "Children": [{
                "id": 1, "Text": "Intel Core i7-12700K", "Min": "", "Max": "", "Value": "", "ImageURL": "images/cpu.png",
                "Children": [{
                    "id": 2, "Text": "Temperatures", "Min": "", "Max": "", "Value": "", "ImageURL": "",
                    "Children": [
                        {"id": 3, "Text": "CPU Package: 72.0 °C", "Min": "", "Max": "", "Value": "72.0 °C", "ImageURL": "", "Children": []},
                        {"id": 4, "Text": "CPU Core #1: 70.0 °C", "Min": "", "Max": "", "Value": "70.0 °C", "ImageURL": "", "Children": []},
                        {"id": 5, "Text": "Core #1 Distance to TjMax: 30.0 °C", "Min": "", "Max": "", "Value": "30.0 °C", "ImageURL": "", "Children": []},
                        {"id": 6, "Text": "Core #2 Distance to TjMax: 32.0 °C", "Min": "", "Max": "", "Value": "32.0 °C", "ImageURL": "", "Children": []}
                    ]
                }]
            }]
        }"#;
        let snapshot = parse_lhm_json(json).expect("Should parse");
        // Only real sensors: package + core #1 (TjMax entries filtered out)
        assert_eq!(snapshot.cpu_cores.len(), 1, "TjMax entries should be filtered");
        assert!(snapshot.cpu_cores[0].name.contains("Core #1"));
        assert!(snapshot.cpu_package.is_some());
    }

    #[test]
    fn motherboard_cpu_sensor_does_not_inflate_max_temp() {
        // Motherboard has a "CPU" sensor reading 92°C (socket sensor)
        // Real CPU package is 72°C — max_temp should be 72, not 92
        let json = r#"{
            "id": 0, "Text": "Sensor", "Min": "", "Max": "", "Value": "", "ImageURL": "",
            "Children": [
                {
                    "id": 1, "Text": "Intel Core i7-12700K", "Min": "", "Max": "", "Value": "", "ImageURL": "images/cpu.png",
                    "Children": [{
                        "id": 2, "Text": "Temperatures", "Min": "", "Max": "", "Value": "", "ImageURL": "",
                        "Children": [
                            {"id": 3, "Text": "CPU Package: 72.0 °C", "Min": "", "Max": "", "Value": "72.0 °C", "ImageURL": "", "Children": []}
                        ]
                    }]
                },
                {
                    "id": 10, "Text": "Nuvoton NCT6798D", "Min": "", "Max": "", "Value": "", "ImageURL": "images/chip.png",
                    "Children": [{
                        "id": 11, "Text": "Temperatures", "Min": "", "Max": "", "Value": "", "ImageURL": "",
                        "Children": [
                            {"id": 12, "Text": "CPU: 92.0 °C", "Min": "", "Max": "", "Value": "92.0 °C", "ImageURL": "", "Children": []},
                            {"id": 13, "Text": "System: 47.5 °C", "Min": "", "Max": "", "Value": "47.5 °C", "ImageURL": "", "Children": []}
                        ]
                    }]
                }
            ]
        }"#;
        let snapshot = parse_lhm_json(json).expect("Should parse");
        // max_temp should be 72 (CPU package), NOT 92 (motherboard socket sensor)
        assert!(
            (snapshot.max_temp - 72.0).abs() < 0.01,
            "max_temp should be 72.0 from CPU package, got {}",
            snapshot.max_temp
        );
        // The motherboard CPU sensor should still appear but labelled "(socket)"
        let mb_cpu = snapshot.motherboard_temps.iter().find(|s| s.name.contains("socket"));
        assert!(mb_cpu.is_some(), "Motherboard CPU sensor should be present with '(socket)' label");
        assert!((mb_cpu.unwrap().value - 92.0).abs() < 0.01);
        // System sensor at 47.5 should contribute to max_temp (non-CPU motherboard sensor)
        // but it's lower than 72 so max_temp stays at 72
        let system = snapshot.motherboard_temps.iter().find(|s| s.name == "System");
        assert!(system.is_some());
    }
}
