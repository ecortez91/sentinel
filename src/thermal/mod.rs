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

/// Client for polling LibreHardwareMonitor's HTTP JSON endpoint.
pub struct LhmClient {
    url: String,
    client: reqwest::Client,
}

impl LhmClient {
    /// Create a new client pointing at the given LHM URL.
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Poll LHM and return a thermal snapshot, or None if unreachable / parse error.
    pub async fn poll(&self) -> Option<ThermalSnapshot> {
        let resp = self.client.get(&self.url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().await.ok()?;
        parse_lhm_json(&text)
    }
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
                // Track max temp across all temperature sensors
                if sensor.value > snapshot.max_temp {
                    snapshot.max_temp = sensor.value;
                }

                if is_cpu_hardware(&path_str) {
                    // CPU temperatures
                    if name_lower.contains("package") || name_lower.contains("cpu total") {
                        snapshot.cpu_package = Some(sensor.value);
                        if sensor.value > snapshot.max_cpu_temp {
                            snapshot.max_cpu_temp = sensor.value;
                        }
                    } else if name_lower.contains("core") {
                        snapshot.cpu_cores.push(SensorReading {
                            name: sensor.name.clone(),
                            value: sensor.value,
                        });
                        if sensor.value > snapshot.max_cpu_temp {
                            snapshot.max_cpu_temp = sensor.value;
                        }
                    } else {
                        // Other CPU temps
                        if sensor.value > snapshot.max_cpu_temp {
                            snapshot.max_cpu_temp = sensor.value;
                        }
                    }
                } else if is_gpu_hardware(&path_str) {
                    // GPU temperatures
                    if name_lower.contains("hot spot") || name_lower.contains("hotspot") {
                        snapshot.gpu_hotspot = Some(sensor.value);
                    } else if name_lower.contains("gpu") || name_lower.contains("temperature") {
                        snapshot.gpu_temp = Some(sensor.value);
                    }
                    if sensor.value > snapshot.max_gpu_temp {
                        snapshot.max_gpu_temp = sensor.value;
                    }
                } else if is_storage_hardware(&path_str) {
                    snapshot.ssd_temps.push(SensorReading {
                        name: format_storage_name(&sensor.hardware_path, &sensor.name),
                        value: sensor.value,
                    });
                } else {
                    // Motherboard / chipset / other
                    snapshot.motherboard_temps.push(SensorReading {
                        name: sensor.name.clone(),
                        value: sensor.value,
                    });
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
}
