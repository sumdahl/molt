//! Tests for `molt status` (specs/013-status.md). The parsers get fixtures; the readers
//! only read this machine.

use super::*;

use std::process::Command;

#[test]
fn st_02_cpu_times_are_parsed_and_compared() {
    let before = cpu_times("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
    let after = cpu_times("cpu  200 0 100 1100 100 0 0 0 7 7\n").unwrap();
    // 150 busy of 500: idle and iowait are idle; guest is already in user.
    assert_eq!(busy_share(before, after), Some(30.0));
    assert_eq!(busy_share(before, before), None, "no time passed");
    assert_eq!(cpu_times("intr 5\n"), None);
}

#[test]
fn st_02_load_and_uptime_are_parsed() {
    assert_eq!(
        load("0.36 0.68 0.99 2/1317 1592460\n"),
        Some([0.36, 0.68, 0.99])
    );
    assert_eq!(load("garbage"), None);
    assert_eq!(uptime("23839.96 271313.07\n"), Some(23839));
}

#[test]
fn st_03_memory_and_swap_come_from_meminfo() {
    let text = "MemTotal:       16000 kB\nMemFree:  1 kB\nMemAvailable:    6000 kB\n\
                SwapTotal:      2000 kB\nSwapFree:       1500 kB\n";
    let memory = memory(text).unwrap();
    assert_eq!(memory.total, 16000 * 1024);
    assert_eq!(memory.available, 6000 * 1024);
    assert_eq!(memory.used, 10000 * 1024);
    assert_eq!(memory.swap_total, 2000 * 1024);
    assert_eq!(memory.swap_used, 500 * 1024);
    assert_eq!(
        memory_line(&memory),
        "  Memory   9.8 MiB of 15.6 MiB used (62.5%), 5.9 MiB available"
    );
    assert!(
        self::memory("MemTotal: 1 kB\n").is_none(),
        "no MemAvailable"
    );
}

#[test]
fn st_04_only_real_filesystems_are_listed_once_each() {
    let text = "\
/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0
proc /proc proc rw 0 0
tmpfs /tmp tmpfs rw 0 0
/dev/nvme0n1p2 /home btrfs rw,subvol=/@home 0 0
/dev/nvme0n1p1 /boot vfat rw 0 0
server:/export /mnt/nfs nfs4 rw 0 0
gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw 0 0
/dev/sda1 /media/My\\040Disk ext4 rw 0 0
overlay /var/lib/docker/overlay overlay rw 0 0
/dev/loop0 /snap/core/1 squashfs ro 0 0
";
    let mounts = mounts(text);
    let points: Vec<&str> = mounts.iter().map(|m| m.point.as_str()).collect();
    assert_eq!(points, ["/", "/boot", "/media/My Disk", "/mnt/nfs"]);
    assert_eq!(mounts[3].kind, "nfs4");
}

#[test]
fn st_04_disk_sizes_follow_df() {
    let disk = Disk::from_counts("/".to_owned(), "ext4".to_owned(), 4096, 1000, 400, 300);
    assert_eq!(disk.size, Some(1000 * 4096));
    assert_eq!(disk.used, Some(600 * 4096));
    assert_eq!(disk.free, Some(300 * 4096));
    // Used of what an unprivileged user could have: 600 / (600 + 300).
    assert_eq!(
        disk.used_share.map(|s| (s * 10.0).round() / 10.0),
        Some(66.7)
    );
}

#[test]
fn st_04_every_real_disk_here_is_measured() {
    let disks = disks();
    let root = disks
        .iter()
        .find(|disk| disk.point == "/")
        .expect("/ is listed");
    assert!(root.size.is_some_and(|size| size > 0), "{root:?}");
}

#[test]
fn st_05_process_stat_is_parsed_even_with_odd_names() {
    assert_eq!(
        process("42 (a (b) c) Z 7 42 42 0 -1\n"),
        Some(('Z', "a (b) c".to_owned(), 7))
    );
    assert_eq!(
        process("1 (systemd) S 0 1 1\n"),
        Some(('S', "systemd".to_owned(), 0))
    );
    assert_eq!(process("nonsense"), None);
}

#[test]
fn st_05_a_child_that_isnt_reaped_is_a_zombie() {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    let stat = format!("/proc/{pid}/stat");
    for _ in 0..500 {
        if fs::read_to_string(&stat).is_ok_and(|s| process(&s).is_some_and(|p| p.0 == 'Z')) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let found = processes();
    child.wait().unwrap();

    let zombie = found.zombies.iter().find(|z| z.pid == pid).expect("listed");
    assert_eq!(zombie.parent, std::process::id());
    assert!(found.total >= 2);
}

#[test]
fn st_06_nearly_full_disks_point_at_analyze() {
    let full = Disk::from_counts("/data".to_owned(), "ext4".to_owned(), 4096, 100, 5, 5);
    let fine = Disk::from_counts("/".to_owned(), "ext4".to_owned(), 4096, 100, 50, 50);
    assert!(disk_line(&full).contains("(nearly full: try `molt analyze /data`)"));
    assert!(!disk_line(&fine).contains("nearly full"));
    let dead = Disk::unavailable("/mnt/nfs".to_owned(), "nfs4".to_owned());
    assert!(disk_line(&dead).contains("unavailable"));
}
