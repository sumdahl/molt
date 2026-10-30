//! `molt status` (specs/013-status.md): the machine's health, read from `/proc` and
//! `/sys` without root. Changes nothing (ST-01).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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
    let first = read("/proc/stat").as_deref().and_then(cpu_times);
    // The disks are measured while the CPU sample runs.
    let disks = thread::scope(|scope| {
        let disks = scope.spawn(disks);
        thread::sleep(SAMPLE);
        disks.join().unwrap_or_default()
    });
    let second = read("/proc/stat").as_deref().and_then(cpu_times);
    Status {
        cpu: Cpu {
            count: thread::available_parallelism().ok().map(usize::from),
            load: read("/proc/loadavg").as_deref().and_then(load),
            busy: first.zip(second).and_then(|(a, b)| busy_share(a, b)),
            uptime: read("/proc/uptime").as_deref().and_then(uptime),
        },
        memory: read("/proc/meminfo").as_deref().and_then(memory),
        disks,
        processes: processes(),
    }
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
