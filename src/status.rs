//! `molt status` (specs/013-status.md): the machine's health, read from `/proc` and
//! `/sys` without root. Changes nothing (ST-01).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::scan::human_size;

#[cfg(test)]
mod tests;

/// ST-02: how long the CPU sample runs.
const SAMPLE: Duration = Duration::from_millis(200);
/// ST-04: how long a disk may take to answer before it's called unavailable.
const DISK_TIMEOUT: Duration = Duration::from_secs(1);
/// ST-06
const NEARLY_FULL: f64 = 90.0;

/// ST-04: filesystems that hold no files of their own.
const PSEUDO: [&str; 24] = [
    "proc",
    "sysfs",
    "tmpfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "overlay",
    "squashfs",
    "securityfs",
    "pstore",
    "bpf",
    "debugfs",
    "tracefs",
    "mqueue",
    "hugetlbfs",
    "fusectl",
    "configfs",
    "efivarfs",
    "autofs",
    "binfmt_misc",
    "ramfs",
    "nsfs",
    "rpc_pipefs",
];
/// ST-04: shares whose source isn't a device path.
const NETWORK: [&str; 7] = ["nfs", "nfs4", "cifs", "smb3", "fuse.sshfs", "sshfs", "9p"];

#[derive(Debug, Serialize)]
pub struct Status {
    pub cpu: Cpu,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    pub disks: Vec<Disk>,
    pub processes: Processes,
    /// ST-09: bytes read and written per second, per whole disk.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disk_io: Vec<Rate>,
    /// ST-09: bytes received and sent per second, per interface.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<Rate>,
    /// ST-10, ST-11
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sensors: Vec<Sensor>,
    /// ST-12
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub batteries: Vec<Battery>,
    /// ST-13
    pub health: Health,
}

/// Two counters per second: read and written, or received and sent.
#[derive(Debug, Serialize)]
pub struct Rate {
    pub name: String,
    pub first: u64,
    pub second: u64,
}

#[derive(Debug, Serialize)]
pub struct Sensor {
    pub name: String,
    pub temperatures: Vec<Temperature>,
    pub fans: Vec<Fan>,
}

#[derive(Debug, Serialize)]
pub struct Temperature {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub celsius: f64,
    /// Past the sensor's critical (or else maximum) limit.
    pub hot: bool,
}

#[derive(Debug, Serialize)]
pub struct Fan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub rpm: u64,
}

#[derive(Debug, Serialize)]
pub struct Battery {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Full charge now, as a share of the design's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct Health {
    pub score: u8,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Serialize)]
pub struct Reason {
    pub points: i32,
    pub why: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Cpu {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// 1, 5 and 15 minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<[f64; 3]>,
    /// Share of non-idle time over the sample, 0 to 100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy: Option<f64>,
    /// Seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Debug, Serialize)]
pub struct Disk {
    pub point: String,
    pub kind: String,
    /// `None` when the filesystem didn't answer (ST-04).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
    /// What an unprivileged user can still write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_share: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct Processes {
    pub total: usize,
    pub running: usize,
    pub zombies: Vec<Zombie>,
}

#[derive(Debug, Serialize)]
pub struct Zombie {
    pub pid: u32,
    pub name: String,
    pub parent: u32,
}

/// A mount worth listing, from `/proc/self/mounts`.
#[derive(Debug, PartialEq, Eq)]
struct Mount {
    point: String,
    kind: String,
}

/// ST-01: everything, read now. Takes about `SAMPLE`.
pub fn status() -> Status {
    let sample = Sample::now();
    // The disks are measured while the sample runs.
    let disks = thread::scope(|scope| {
        let disks = scope.spawn(disks);
        thread::sleep(SAMPLE);
        disks.join().unwrap_or_default()
    });
    let after = Sample::now();
    let elapsed = after.at.duration_since(sample.at);
    let (first, second) = (sample.cpu, after.cpu);
    let mut status = Status {
        cpu: Cpu {
            count: thread::available_parallelism().ok().map(usize::from),
            load: read("/proc/loadavg").as_deref().and_then(load),
            busy: first.zip(second).and_then(|(a, b)| busy_share(a, b)),
            uptime: read("/proc/uptime").as_deref().and_then(uptime),
        },
        memory: read("/proc/meminfo").as_deref().and_then(memory),
        disks,
        processes: processes(),
        disk_io: rates(&sample.disks, &after.disks, elapsed),
        network: rates(&sample.network, &after.network, elapsed),
        sensors: sensors(Path::new("/sys/class/hwmon")),
        batteries: batteries(Path::new("/sys/class/power_supply")),
        health: Health::default(),
    };
    status.health = health(&status);
    status
}

/// The counters a rate is the change of (ST-02, ST-09).
struct Sample {
    at: Instant,
    cpu: Option<CpuTimes>,
    disks: Vec<(String, (u64, u64))>,
    network: Vec<(String, (u64, u64))>,
}

impl Sample {
    fn now() -> Self {
        // Whole disks are the ones /sys/block lists; partitions aren't there.
        let whole = |name: &str| Path::new("/sys/block").join(name).exists();
        Self {
            at: Instant::now(),
            cpu: read("/proc/stat").as_deref().and_then(cpu_times),
            disks: read("/proc/diskstats")
                .map_or_else(Vec::new, |text| disk_counters(&text, whole)),
            network: read("/proc/net/dev").map_or_else(Vec::new, |text| net_counters(&text)),
        }
    }
}

/// ST-09: bytes read and written per whole disk, leaving out loop, ram and zram devices.
fn disk_counters(text: &str, whole: impl Fn(&str) -> bool) -> Vec<(String, (u64, u64))> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let name = *fields.get(2)?;
            let virtual_disk = ["loop", "ram", "zram"].iter().any(|p| name.starts_with(p));
            if virtual_disk || !whole(name) {
                return None;
            }
            let sectors = |i: usize| fields.get(i)?.parse::<u64>().ok();
            // Sectors are 512 bytes here, whatever the device's own size.
            Some((name.to_owned(), (sectors(5)? * 512, sectors(9)? * 512)))
        })
        .collect()
}

/// ST-09: bytes received and sent per interface, except `lo`.
fn net_counters(text: &str) -> Vec<(String, (u64, u64))> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let name = name.trim();
            let fields: Vec<u64> = counters
                .split_whitespace()
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            if name == "lo" {
                return None;
            }
            Some((name.to_owned(), (*fields.first()?, *fields.get(8)?)))
        })
        .collect()
}

/// Per second, for each name in both samples.
fn rates(
    before: &[(String, (u64, u64))],
    after: &[(String, (u64, u64))],
    elapsed: Duration,
) -> Vec<Rate> {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return Vec::new();
    }
    let per_second = |delta: u64| {
        // Display precision only; the result is rounded to whole bytes.
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let rate = (delta as f64 / seconds).round() as u64;
        rate
    };
    after
        .iter()
        .filter_map(|(name, (first, second))| {
            let (_, (was_first, was_second)) = before.iter().find(|(n, _)| n == name)?;
            Some(Rate {
                name: name.clone(),
                first: per_second(first.saturating_sub(*was_first)),
                second: per_second(second.saturating_sub(*was_second)),
            })
        })
        .collect()
}

/// ST-10, ST-11: each hwmon sensor under `root` that has a reading, by name.
fn sensors(root: &Path) -> Vec<Sensor> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs.iter()
        .filter_map(|dir| {
            let value = |file: &str| read_trimmed(&dir.join(file));
            let number = |file: &str| value(file).and_then(|v| v.parse::<i64>().ok());
            let inputs = |prefix: &str| -> Vec<u32> {
                let mut found: Vec<u32> = fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        name.strip_prefix(prefix)?
                            .strip_suffix("_input")?
                            .parse()
                            .ok()
                    })
                    .collect();
                found.sort_unstable();
                found
            };
            let temperatures: Vec<Temperature> = inputs("temp")
                .into_iter()
                .filter_map(|n| {
                    let milli = number(&format!("temp{n}_input"))?;
                    let limit = number(&format!("temp{n}_crit"))
                        .or_else(|| number(&format!("temp{n}_max")))
                        .filter(|&limit| limit > 0);
                    Some(Temperature {
                        label: value(&format!("temp{n}_label")),
                        celsius: millidegrees(milli),
                        hot: limit.is_some_and(|limit| milli >= limit),
                    })
                })
                .collect();
            let fans: Vec<Fan> = inputs("fan")
                .into_iter()
                .filter_map(|n| {
                    Some(Fan {
                        label: value(&format!("fan{n}_label")),
                        rpm: u64::try_from(number(&format!("fan{n}_input"))?).ok()?,
                    })
                })
                .collect();
            let name = value("name")?;
            (!temperatures.is_empty() || !fans.is_empty()).then_some(Sensor {
                name,
                temperatures,
                fans,
            })
        })
        .collect()
}

fn millidegrees(milli: i64) -> f64 {
    // Sensor values are far inside f64's exact range.
    #[allow(clippy::cast_precision_loss)]
    let celsius = milli as f64 / 1000.0;
    celsius
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.trim().to_owned()).filter(|text| !text.is_empty())
}

/// ST-12: each power supply under `root` whose type is `Battery`.
fn batteries(root: &Path) -> Vec<Battery> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<Battery> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let dir = entry.path();
            let value = |file: &str| read_trimmed(&dir.join(file));
            let number = |file: &str| value(file).and_then(|v| v.parse::<u64>().ok());
            if value("type").as_deref() != Some("Battery") {
                return None;
            }
            let pair = |kind: &str| {
                let full = number(&format!("{kind}_full"))?;
                let design = number(&format!("{kind}_full_design")).filter(|&d| d > 0)?;
                Some(percent(full, design))
            };
            Some(Battery {
                name: entry.file_name().to_string_lossy().into_owned(),
                charge: number("capacity").and_then(|c| u8::try_from(c).ok()),
                status: value("status"),
                health: pair("energy").or_else(|| pair("charge")),
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// ST-13: 100, less a listed number of points for each listed problem, never below 0.
fn health(status: &Status) -> Health {
    let mut reasons = Vec::new();
    let mut lose = |points: i32, why: String| {
        reasons.push(Reason {
            points: -points,
            why,
        })
    };
    for disk in &status.disks {
        match disk.used_share.map(f64::ceil) {
            Some(share) if share >= 90.0 => lose(15, format!("{} is {share:.0}% full", disk.point)),
            Some(share) if share >= 80.0 => lose(5, format!("{} is {share:.0}% full", disk.point)),
            _ => {}
        }
    }
    if let Some(memory) = &status.memory {
        let available = percent(memory.available, memory.total.max(1));
        if available < 10.0 {
            lose(15, format!("only {available:.0}% of memory is available"));
        } else if available < 20.0 {
            lose(5, format!("only {available:.0}% of memory is available"));
        }
        if memory.swap_total > 0 && percent(memory.swap_used, memory.swap_total) > 50.0 {
            lose(5, "over half the swap is in use".to_owned());
        }
    }
    if let (Some([one, ..]), Some(count)) = (status.cpu.load, status.cpu.count) {
        // CPU counts are small; f64 holds them exactly.
        #[allow(clippy::cast_precision_loss)]
        let limit = 2.0 * count as f64;
        if one > limit {
            lose(
                10,
                format!("the load ({one:.2}) is over twice the {count} CPUs"),
            );
        }
    }
    let hot: Vec<&str> = status
        .sensors
        .iter()
        .filter(|sensor| sensor.temperatures.iter().any(|t| t.hot))
        .map(|sensor| sensor.name.as_str())
        .collect();
    if !hot.is_empty() {
        lose(15, format!("running hot: {}", hot.join(", ")));
    }
    let zombies = status.processes.zombies.len();
    if zombies > 0 {
        let points = i32::try_from(zombies.saturating_mul(2))
            .unwrap_or(i32::MAX)
            .min(10);
        lose(points, format!("{zombies} zombie processes"));
    }
    for battery in &status.batteries {
        if let Some(health) = battery.health.filter(|&h| h < 60.0) {
            lose(
                10,
                format!("{} holds {health:.0}% of its design charge", battery.name),
            );
        }
    }
    let lost: i32 = reasons.iter().map(|reason| reason.points).sum();
    Health {
        score: u8::try_from((100 + lost).max(0)).unwrap_or(0),
        reasons,
    }
}

/// ST-10: the hottest reading of a sensor, and its fans.
fn sensor_line(sensor: &Sensor) -> String {
    let mut line = format!("  {:<20}", sensor.name);
    let hottest = sensor
        .temperatures
        .iter()
        .max_by(|a, b| a.celsius.total_cmp(&b.celsius));
    if let Some(reading) = hottest {
        line.push_str(&format!(" {:.1} °C", reading.celsius));
        if reading.hot {
            line.push_str(" (hot)");
        }
    }
    for fan in &sensor.fans {
        let label = fan.label.as_deref().unwrap_or("fan");
        match fan.rpm {
            0 => line.push_str(&format!("  {label} stopped")),
            rpm => line.push_str(&format!("  {label} {rpm} RPM")),
        }
    }
    line
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Busy and total jiffies from the first line of `/proc/stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    busy: u64,
    total: u64,
}

fn cpu_times(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    // user nice system idle iowait irq softirq steal; guest time is already in user.
    let total: u64 = fields.iter().take(8).sum();
    let idle = fields.get(3)? + fields.get(4).unwrap_or(&0);
    Some(CpuTimes {
        busy: total - idle,
        total,
    })
}

fn busy_share(before: CpuTimes, after: CpuTimes) -> Option<f64> {
    let total = after.total.checked_sub(before.total).filter(|&t| t > 0)?;
    let busy = after.busy.saturating_sub(before.busy);
    Some(percent(busy, total))
}

fn load(loadavg: &str) -> Option<[f64; 3]> {
    let mut fields = loadavg.split_whitespace().map(str::parse::<f64>);
    Some([
        fields.next()?.ok()?,
        fields.next()?.ok()?,
        fields.next()?.ok()?,
    ])
}

fn uptime(text: &str) -> Option<u64> {
    let seconds = text.split_whitespace().next()?;
    seconds.split('.').next()?.parse().ok()
}

/// ST-03
fn memory(meminfo: &str) -> Option<Memory> {
    let kib: HashMap<&str, u64> = meminfo
        .lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let value = rest.split_whitespace().next()?.parse().ok()?;
            Some((key, value))
        })
        .collect();
    let total = kib.get("MemTotal")? * 1024;
    let available = kib.get("MemAvailable")? * 1024;
    let swap_total = kib.get("SwapTotal").copied().unwrap_or(0) * 1024;
    let swap_free = kib.get("SwapFree").copied().unwrap_or(0) * 1024;
    Some(Memory {
        total,
        available,
        used: total.saturating_sub(available),
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free),
    })
}

/// ST-04: real filesystems, each device once, at its shortest mount point.
fn mounts(text: &str) -> Vec<Mount> {
    let mut found: Vec<(String, Mount)> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (source, point, kind) = (fields.next()?, fields.next()?, fields.next()?);
            let real =
                !PSEUDO.contains(&kind) && (source.starts_with('/') || NETWORK.contains(&kind));
            real.then(|| {
                let mount = Mount {
                    point: unescape(point),
                    kind: kind.to_owned(),
                };
                (source.to_owned(), mount)
            })
        })
        .collect();
    found.sort_by_key(|(_, mount)| mount.point.len());
    let mut seen = HashSet::new();
    let mut mounts: Vec<Mount> = found
        .into_iter()
        .filter(|(source, _)| seen.insert(source.clone()))
        .map(|(_, mount)| mount)
        .collect();
    mounts.sort_by(|a, b| a.point.cmp(&b.point));
    mounts
}

/// `/proc/self/mounts` writes space, tab, newline and backslash as octal escapes.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes.get(i + 1..i + 4).and_then(|digits| {
            let digits = std::str::from_utf8(digits).ok()?;
            u8::from_str_radix(digits, 8).ok()
        });
        match (bytes[i], octal) {
            (b'\\', Some(byte)) => {
                out.push(byte);
                i += 4;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// ST-04: each disk measured on its own thread; one that doesn't answer in time is
/// unavailable, and its thread is left to finish (or not) on its own.
fn disks() -> Vec<Disk> {
    let Some(text) = read("/proc/self/mounts") else {
        return Vec::new();
    };
    let pending: Vec<_> = mounts(&text)
        .into_iter()
        .map(|mount| {
            let (send, answer) = mpsc::channel();
            let point = mount.point.clone();
            thread::spawn(move || send.send(rustix::fs::statvfs(point.as_str())));
            (mount, answer)
        })
        .collect();
    pending
        .into_iter()
        .map(|(mount, answer)| match answer.recv_timeout(DISK_TIMEOUT) {
            Ok(Ok(vfs)) => Disk::from_counts(
                mount.point,
                mount.kind,
                vfs.f_frsize,
                vfs.f_blocks,
                vfs.f_bfree,
                vfs.f_bavail,
            ),
            _ => Disk::unavailable(mount.point, mount.kind),
        })
        .collect()
}

impl Disk {
    /// As `df`: used is all blocks minus free ones; the share is of what a user could use.
    fn from_counts(
        point: String,
        kind: String,
        block: u64,
        blocks: u64,
        free: u64,
        avail: u64,
    ) -> Self {
        let used = blocks.saturating_sub(free);
        Self {
            point,
            kind,
            size: Some(blocks * block),
            used: Some(used * block),
            free: Some(avail * block),
            used_share: (used + avail > 0).then(|| percent(used, used + avail)),
        }
    }

    fn unavailable(point: String, kind: String) -> Self {
        Self {
            point,
            kind,
            size: None,
            used: None,
            free: None,
            used_share: None,
        }
    }
}

/// ST-05: every process in `/proc`, counted by state, with the zombies named.
fn processes() -> Processes {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Processes::default();
    };
    let mut found = Processes::default();
    for entry in entries.filter_map(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        // A process that ended mid-read is just gone.
        let Some((state, name, parent)) = read(&format!("/proc/{pid}/stat"))
            .as_deref()
            .and_then(process)
        else {
            continue;
        };
        found.total += 1;
        match state {
            'R' => found.running += 1,
            'Z' => found.zombies.push(Zombie { pid, name, parent }),
            _ => {}
        }
    }
    found.zombies.sort_by_key(|zombie| zombie.pid);
    found
}

/// State, name and parent from `/proc/<pid>/stat`. The name is in parentheses and may
/// hold anything, parentheses too, so it ends at the last `)`.
fn process(stat: &str) -> Option<(char, String, u32)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_owned();
    let mut rest = stat.get(close + 1..)?.split_whitespace();
    let state = rest.next()?.chars().next()?;
    let parent = rest.next()?.parse().ok()?;
    Some((state, name, parent))
}

fn percent(part: u64, whole: u64) -> f64 {
    // Display precision only.
    #[allow(clippy::cast_precision_loss)]
    let share = part as f64 * 100.0 / whole as f64;
    share
}

fn memory_line(memory: &Memory) -> String {
    format!(
        "  Memory   {} of {} used ({:.1}%), {} available",
        human_size(memory.used),
        human_size(memory.total),
        percent(memory.used, memory.total.max(1)),
        human_size(memory.available)
    )
}

fn disk_line(disk: &Disk) -> String {
    let (Some(size), Some(used), Some(free), Some(share)) =
        (disk.size, disk.used, disk.free, disk.used_share)
    else {
        return format!("  {:<24} {:<8} unavailable", disk.point, disk.kind);
    };
    let mut line = format!(
        "  {:<24} {:<8} {:>10} of {:>10} used ({:.0}%), {} free",
        disk.point,
        disk.kind,
        human_size(used),
        human_size(size),
        // Rounded up, as `df` does, so the two agree.
        share.ceil(),
        human_size(free)
    );
    if share >= NEARLY_FULL {
        line.push_str(&format!(
            "  (nearly full: try `molt analyze {}`)",
            disk.point
        ));
    }
    line
}

/// ST-06
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "CPU")?;
        let cpu = &self.cpu;
        if let Some(count) = cpu.count {
            write!(f, "  {count} CPUs")?;
        }
        if let Some(busy) = cpu.busy {
            write!(f, ", {busy:.0}% busy")?;
        }
        if let Some([one, five, fifteen]) = cpu.load {
            write!(f, ", load {one:.2} {five:.2} {fifteen:.2}")?;
        }
        if let Some(uptime) = cpu.uptime {
            write!(f, ", up {}", duration(uptime))?;
        }
        writeln!(f)?;

        writeln!(f, "\nMemory")?;
        match &self.memory {
            Some(memory) => {
                writeln!(f, "{}", memory_line(memory))?;
                if memory.swap_total == 0 {
                    writeln!(f, "  Swap     none")?;
                } else {
                    writeln!(
                        f,
                        "  Swap     {} of {} used",
                        human_size(memory.swap_used),
                        human_size(memory.swap_total)
                    )?;
                }
            }
            None => writeln!(f, "  unavailable")?,
        }

        writeln!(f, "\nDisks")?;
        for disk in &self.disks {
            writeln!(f, "{}", disk_line(disk))?;
        }

        let processes = &self.processes;
        writeln!(f, "\nProcesses")?;
        writeln!(
            f,
            "  {} processes, {} running",
            processes.total, processes.running
        )?;
        for zombie in &processes.zombies {
            writeln!(
                f,
                "  zombie {} ({}), not reaped by its parent {}",
                zombie.pid, zombie.name, zombie.parent
            )?;
        }

        if !self.disk_io.is_empty() || !self.network.is_empty() {
            writeln!(f, "\nI/O per second")?;
            for rate in &self.disk_io {
                writeln!(
                    f,
                    "  {:<20} read {:>10}  written {:>10}",
                    rate.name,
                    human_size(rate.first),
                    human_size(rate.second)
                )?;
            }
            // Idle interfaces (bridges, veths) would bury the busy ones; --json has all.
            let (busy, idle): (Vec<&Rate>, Vec<&Rate>) = self
                .network
                .iter()
                .partition(|rate| rate.first + rate.second > 0);
            for rate in busy {
                writeln!(
                    f,
                    "  {:<20} in   {:>10}  out     {:>10}",
                    rate.name,
                    human_size(rate.first),
                    human_size(rate.second)
                )?;
            }
            if !idle.is_empty() {
                writeln!(f, "  {} idle network interfaces not shown", idle.len())?;
            }
        }
        if !self.sensors.is_empty() {
            writeln!(f, "\nSensors")?;
            for sensor in &self.sensors {
                writeln!(f, "{}", sensor_line(sensor))?;
            }
        }
        for battery in &self.batteries {
            writeln!(f, "\nBattery")?;
            write!(f, "  {}", battery.name)?;
            if let Some(charge) = battery.charge {
                write!(f, " {charge}%")?;
            }
            if let Some(status) = &battery.status {
                write!(f, ", {}", status.to_lowercase())?;
            }
            if let Some(health) = battery.health {
                write!(f, ", holds {health:.0}% of its design charge")?;
            }
            writeln!(f)?;
        }

        writeln!(f, "\nHealth {}/100", self.health.score)?;
        for reason in &self.health.reasons {
            writeln!(f, "  {}: {}", reason.points, reason.why)?;
        }
        Ok(())
    }
}

/// `3d 4h`, `4h 12m` or `12m`.
fn duration(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}
