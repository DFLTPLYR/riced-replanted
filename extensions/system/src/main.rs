use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use riced_extension_sdk::Plugin;
use serde_json::{Map, Value, json};
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System};

fn main() -> std::io::Result<()> {
    Plugin::new("system")
        .state([
            "host",
            "time",
            "cpu",
            "memory",
            "disks",
            "gpus",
            "processes",
        ])
        .serve(|context| {
            let mut system = System::new();
            let mut last = Value::Null;
            loop {
                if context.requested().is_empty() {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }

                // Only refresh the sysinfo groups something asked for; `/sys` reads are
                // below and also gated on the request.
                let mut kind = RefreshKind::nothing();
                if context.wants("cpu") {
                    kind = kind.with_cpu(CpuRefreshKind::everything());
                }
                if context.wants("memory") {
                    kind = kind.with_memory(MemoryRefreshKind::everything());
                }
                if context.wants("processes") {
                    kind = kind.with_processes(ProcessRefreshKind::everything());
                }
                system.refresh_specifics(kind);

                let mut store = Map::new();
                if context.wants("host") {
                    store.insert(
                        "host".into(),
                        json!({
                            "name": System::host_name(),
                            "os": System::name(),
                            "version": System::os_version(),
                            "kernel": System::kernel_version(),
                            "uptime": System::uptime(),
                            "boot_time": System::boot_time(),
                        }),
                    );
                }
                if context.wants("time") {
                    store.insert("time".into(), clock());
                }
                if context.wants("cpu") {
                    let cpus = system.cpus();
                    let load = System::load_average();
                    store.insert(
                        "cpu".into(),
                        json!({
                            "usage": system.global_cpu_usage(),
                            "cores": cpus.len(),
                            "physical": system.physical_core_count(),
                            "brand": cpus.first().map(|cpu| cpu.brand()).unwrap_or(""),
                            "frequency": cpus.iter().map(|cpu| cpu.frequency()).max().unwrap_or(0),
                            "load1": load.one,
                            "load5": load.five,
                            "load15": load.fifteen,
                        }),
                    );
                }
                if context.wants("memory") {
                    store.insert(
                        "memory".into(),
                        json!({
                            "total": system.total_memory(),
                            "used": system.used_memory(),
                            "available": system.available_memory(),
                            "swap_total": system.total_swap(),
                            "swap_used": system.used_swap(),
                        }),
                    );
                }
                if context.wants("disks") {
                    let disks = Disks::new_with_refreshed_list();
                    store.insert(
                        "disks".into(),
                        json!(disks
                            .list()
                            .iter()
                            .filter(|disk| disk.total_space() > 0)
                            .map(|disk| json!({
                                "mount": disk.mount_point().to_string_lossy(),
                                "fs": disk.file_system().to_string_lossy(),
                                "total": disk.total_space(),
                                "available": disk.available_space(),
                                "used": disk.total_space().saturating_sub(disk.available_space()),
                            }))
                            .collect::<Vec<_>>()),
                    );
                }
                if context.wants("gpus") {
                    store.insert("gpus".into(), Value::Array(gpus()));
                }
                if context.wants("processes") {
                    store.insert("processes".into(), json!(system.processes().len()));
                }
                let store = Value::Object(store);
                if store != last {
                    context.publish(store.clone());
                    last = store;
                }
                thread::sleep(Duration::from_secs(1));
            }
        })
}

fn clock() -> Value {
    let Ok(seconds) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return Value::Null;
    };
    let Ok(epoch) = libc::time_t::try_from(seconds.as_secs()) else {
        return Value::Null;
    };
    // localtime_r fills a broken-down local time; no allocation, thread-safe.
    let mut time: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&epoch, &mut time) }.is_null() {
        return Value::Null;
    }
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    json!({
        "epoch": seconds.as_secs(),
        "time": format!("{:02}:{:02}", time.tm_hour, time.tm_min),
        "date": format!("{:04}-{:02}-{:02}", time.tm_year + 1900, time.tm_mon + 1, time.tm_mday),
        "weekday": WEEKDAYS.get(time.tm_wday as usize).copied().unwrap_or(""),
    })
}

fn gpus() -> Vec<Value> {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_card(path))
        .collect::<Vec<_>>();
    cards.sort();
    cards.iter().filter_map(|card| gpu(card)).collect()
}

fn is_card(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("card"))
        .is_some_and(|index| !index.is_empty() && index.chars().all(|c| c.is_ascii_digit()))
}

fn gpu(card: &Path) -> Option<Value> {
    let device = card.join("device");
    let uevent = fs::read_to_string(device.join("uevent")).unwrap_or_default();
    let driver = field(&uevent, "DRIVER=");
    let pci = field(&uevent, "PCI_ID=");
    let name = fs::read_to_string(device.join("product_name"))
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| match (driver, pci) {
            (Some(driver), Some(pci)) => Some(format!("{driver} {pci}")),
            (Some(driver), None) => Some(driver.to_owned()),
            _ => card
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        })?;
    Some(json!({
        "name": name,
        "busy": number(&device.join("gpu_busy_percent")),
        "vram_used": number(&device.join("mem_info_vram_used")),
        "vram_total": number(&device.join("mem_info_vram_total")),
        "temp": temperature(&device),
        "clock": clock_mhz(&device.join("pp_dpm_sclk")),
    }))
}

fn field<'a>(uevent: &'a str, prefix: &str) -> Option<&'a str> {
    uevent
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn number(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn temperature(device: &Path) -> Option<u64> {
    let hwmon = fs::read_dir(device.join("hwmon")).ok()?;
    for entry in hwmon.flatten() {
        if let Some(millidegrees) = number(&entry.path().join("temp1_input")) {
            return Some(millidegrees / 1000);
        }
    }
    None
}

fn clock_mhz(path: &Path) -> Option<u64> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .find(|line| line.trim_end().ends_with('*'))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.trim_end_matches("Mhz").parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_active_clock_line() {
        let path = std::env::temp_dir().join(format!("riced-sclk-{}", std::process::id()));
        fs::write(&path, "0: 300Mhz\n1: 800Mhz *\n2: 2100Mhz\n").unwrap();
        assert_eq!(clock_mhz(&path), Some(800));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn reads_uevent_fields() {
        let uevent = "DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:731F\n";
        assert_eq!(field(uevent, "DRIVER="), Some("amdgpu"));
        assert_eq!(field(uevent, "PCI_ID="), Some("1002:731F"));
        assert_eq!(field(uevent, "MISSING="), None);
    }
}
