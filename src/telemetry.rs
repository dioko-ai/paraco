//! Small, read-only Linux host telemetry snapshot.
//!
//! This module deliberately reads aggregate kernel counters and socket tables
//! only. It does not inspect processes, environment variables, or command lines.

/// Return a point-in-time host telemetry snapshot on Linux.
#[cfg(target_os = "linux")]
pub fn snapshot() -> Result<serde_json::Value, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let sampled_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_millis();
    let cpu_text = read_required("/proc/stat")?;
    let (cpu_total, cpu_idle) = parse_cpu(&cpu_text)?;
    let memory_text = read_required("/proc/meminfo")?;
    let memory = parse_memory(&memory_text)?;
    let load_text = read_required("/proc/loadavg")?;
    let load = parse_load(&load_text)?;
    let uptime_text = read_required("/proc/uptime")?;
    let uptime = parse_uptime(&uptime_text)?;
    let hostname = read_required("/proc/sys/kernel/hostname")?
        .trim()
        .to_owned();
    if hostname.is_empty() {
        return Err("/proc/sys/kernel/hostname is empty".into());
    }
    let cores = cpu_text
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu")
                .is_some_and(|tail| tail.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count()
        .max(1);
    let (ports, warnings) = read_ports();

    Ok(serde_json::json!({
        "sampledAt": sampled_at,
        "hostname": hostname,
        "platform": "linux",
        "cores": cores,
        "cpu": { "total": cpu_total, "idle": cpu_idle },
        "memory": memory,
        "load": load,
        "uptime": uptime,
        "ports": ports,
        "warnings": warnings,
    }))
}

/// Non-Linux hosts do not expose the procfs contract used by this module.
#[cfg(not(target_os = "linux"))]
pub fn snapshot() -> Result<serde_json::Value, String> {
    Err("System monitoring is currently available on Linux.".into())
}

#[cfg(target_os = "linux")]
fn read_required(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))
}

/// Parse the aggregate `cpu` row. Guest and guest_nice are excluded because
/// Linux already includes them in user and nice respectively. Idle includes
/// both idle and iowait ticks.
#[cfg(target_os = "linux")]
fn parse_cpu(contents: &str) -> Result<(u64, u64), String> {
    let line = contents
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| "`cpu` aggregate row is missing from /proc/stat".to_owned())?;
    let values = line
        .split_whitespace()
        .skip(1)
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| "invalid CPU tick count".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 4 {
        return Err("aggregate CPU row has fewer than four counters".into());
    }
    // user, nice, system, idle, iowait, irq, softirq, steal. The first four
    // counters are required; older kernels may omit later counters.
    let total = values.iter().take(8).try_fold(0u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or_else(|| "aggregate CPU tick count overflow".to_owned())
    })?;
    let idle = values[3]
        .checked_add(values.get(4).copied().unwrap_or(0))
        .ok_or_else(|| "aggregate CPU idle tick count overflow".to_owned())?;
    Ok((total, idle))
}

#[cfg(target_os = "linux")]
fn parse_memory(contents: &str) -> Result<serde_json::Value, String> {
    use std::collections::HashMap;

    let mut entries = HashMap::new();
    for line in contents.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let Some(value) = fields.next() else { continue };
        let amount = value
            .parse::<u64>()
            .map_err(|_| format!("invalid /proc/meminfo value for {key}"))?;
        let unit = fields.next().unwrap_or("kB");
        let bytes = match unit {
            "kB" => amount
                .checked_mul(1024)
                .ok_or_else(|| format!("/proc/meminfo value for {key} overflows bytes"))?,
            "B" => amount,
            _ => return Err(format!("unsupported /proc/meminfo unit for {key}")),
        };
        entries.insert(key, bytes);
    }
    let get = |name: &str| {
        entries
            .get(name)
            .copied()
            .ok_or_else(|| format!("{name} is missing from /proc/meminfo"))
    };
    Ok(serde_json::json!({
        "total": get("MemTotal")?,
        "available": get("MemAvailable")?,
        "swapTotal": get("SwapTotal")?,
        "swapFree": get("SwapFree")?,
    }))
}

#[cfg(target_os = "linux")]
fn parse_load(contents: &str) -> Result<[f64; 3], String> {
    let mut values = contents.split_whitespace();
    let mut next = || {
        values
            .next()
            .ok_or_else(|| "fewer than three values in /proc/loadavg".to_owned())?
            .parse::<f64>()
            .map_err(|_| "invalid load average in /proc/loadavg".to_owned())
    };
    Ok([next()?, next()?, next()?])
}

#[cfg(target_os = "linux")]
fn parse_uptime(contents: &str) -> Result<f64, String> {
    contents
        .split_whitespace()
        .next()
        .ok_or_else(|| "uptime is missing from /proc/uptime".to_owned())?
        .parse::<f64>()
        .map_err(|_| "invalid uptime in /proc/uptime".to_owned())
}

#[cfg(target_os = "linux")]
fn read_ports() -> (Vec<serde_json::Value>, Vec<String>) {
    const LIMIT: usize = 2048;
    let tables = [
        ("/proc/net/tcp", "tcp", "0A"),
        ("/proc/net/tcp6", "tcp6", "0A"),
        ("/proc/net/udp", "udp", "07"),
        ("/proc/net/udp6", "udp6", "07"),
    ];
    let mut ports = Vec::new();
    let mut warnings = Vec::new();
    let mut truncated = false;
    for (path, protocol, state) in tables {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) => {
                warnings.push(format!("cannot read {path}: {error}"));
                continue;
            }
        };
        for line in contents.lines().skip(1) {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let local = fields.get(1).copied();
            let row_state = fields.get(3).copied();
            if row_state != Some(state) {
                continue;
            }
            let Some((address, port)) =
                local.and_then(|value| parse_socket_address(value, protocol))
            else {
                warnings.push(format!("malformed socket row in {path}"));
                continue;
            };
            if ports.len() == LIMIT {
                truncated = true;
                break;
            }
            ports.push(serde_json::json!({
                "address": address,
                "port": port,
                "protocol": protocol,
                "state": if protocol.starts_with("tcp") { "listening" } else { "bound" },
            }));
        }
        if truncated {
            break;
        }
    }
    if truncated {
        warnings.push(format!("port list truncated at {LIMIT} entries"));
    }
    (ports, warnings)
}

#[cfg(target_os = "linux")]
fn parse_socket_address(value: &str, protocol: &str) -> Option<(String, u16)> {
    use std::net::{Ipv4Addr, Ipv6Addr};

    let (address_hex, port_hex) = value.split_once(':')?;
    let port = u16::from_str_radix(port_hex, 16).ok()?;
    if protocol.ends_with('6') {
        if address_hex.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (index, word) in address_hex.as_bytes().chunks_exact(8).enumerate() {
            let word = std::str::from_utf8(word).ok()?;
            let value = u32::from_str_radix(word, 16).ok()?;
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        Some((Ipv6Addr::from(bytes).to_string(), port))
    } else {
        if address_hex.len() != 8 {
            return None;
        }
        let value = u32::from_str_radix(address_hex, 16).ok()?;
        Some((Ipv4Addr::from(value.to_le_bytes()).to_string(), port))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::parse_cpu;

    #[test]
    fn parses_ipv4_and_port_in_proc_byte_order() {
        let parsed = super::parse_socket_address("0100007F:1F90", "tcp");
        assert_eq!(parsed, Some(("127.0.0.1".into(), 8080)));
    }

    #[test]
    fn parses_ipv6_words_in_proc_byte_order() {
        let parsed = super::parse_socket_address("00000000000000000000000001000000:0050", "tcp6");
        assert_eq!(parsed, Some(("::1".into(), 80)));
    }

    #[test]
    fn aggregate_cpu_excludes_guest_duplicates() {
        let (total, idle) =
            parse_cpu("cpu 100 20 30 400 5 6 7 8 900 40\n").expect("valid aggregate CPU row");
        assert_eq!(total, 576);
        assert_eq!(idle, 405);
    }
}
