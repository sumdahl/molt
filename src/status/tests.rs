//! Tests for `onyx status` (specs/013-status.md). The parsers get fixtures; the readers
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
    assert!(disk_line(&full).contains("(nearly full: try `onyx analyze /data`)"));
    assert!(!disk_line(&fine).contains("nearly full"));
    let dead = Disk::unavailable("/mnt/nfs".to_owned(), "nfs4".to_owned());
    assert!(disk_line(&dead).contains("unavailable"));
}

fn write(path: &std::path::Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn st_09_disk_counters_are_whole_disks_in_bytes() {
    let text = " 259 0 nvme0n1 10 0 2000 0 20 0 4000 0 0 0 0\n \
                259 1 nvme0n1p1 1 0 100 0 1 0 100 0 0 0 0\n \
                7 0 loop0 5 0 999 0 0 0 0 0 0 0 0\n \
                251 0 zram0 5 0 999 0 5 0 999 0 0 0 0\n";
    let whole = |name: &str| !name.ends_with("p1");
    let counters = disk_counters(text, whole);
    assert_eq!(counters, [("nvme0n1".to_owned(), (2000 * 512, 4000 * 512))]);
}

#[test]
fn st_09_network_counters_leave_out_lo() {
    let text = "Inter-|   Receive |  Transmit\n face |bytes packets|bytes\n    \
                lo: 100 1 0 0 0 0 0 0 100 1 0 0 0 0 0 0\n  \
                wlan0: 5000 9 0 0 0 0 0 0 700 3 0 0 0 0 0 0\n";
    assert_eq!(net_counters(text), [("wlan0".to_owned(), (5000, 700))]);
}

#[test]
fn st_09_rates_are_per_second() {
    let before = vec![("sda".to_owned(), (1000, 0))];
    let after = vec![("sda".to_owned(), (3000, 500)), ("new".to_owned(), (9, 9))];
    let rates = rates(&before, &after, Duration::from_millis(500));
    assert_eq!(rates.len(), 1, "a device seen only once has no rate");
    assert_eq!(
        (rates[0].name.as_str(), rates[0].first, rates[0].second),
        ("sda", 4000, 1000)
    );
}

#[test]
fn st_10_st_11_sensors_read_temperatures_fans_and_limits() {
    let dir = tempfile::TempDir::new().unwrap();
    let cpu = dir.path().join("hwmon0");
    write(&cpu.join("name"), "coretemp\n");
    write(&cpu.join("temp1_input"), "45000\n");
    write(&cpu.join("temp1_label"), "Package id 0\n");
    write(&cpu.join("temp1_crit"), "100000\n");
    write(&cpu.join("temp2_input"), "101000\n");
    write(&cpu.join("temp2_crit"), "100000\n");
    let nvme = dir.path().join("hwmon1");
    write(&nvme.join("name"), "nvme\n");
    write(&nvme.join("temp1_input"), "80000\n");
    write(&nvme.join("temp1_max"), "75000\n");
    write(&nvme.join("fan1_input"), "0\n");
    write(&dir.path().join("hwmon2/name"), "AC\n");

    let sensors = sensors(dir.path());

    assert_eq!(sensors.len(), 2, "a sensor with no readings is left out");
    let cpu = &sensors[0];
    assert_eq!(cpu.name, "coretemp");
    assert_eq!(cpu.temperatures[0].label.as_deref(), Some("Package id 0"));
    assert!((cpu.temperatures[0].celsius - 45.0).abs() < 0.01);
    assert!(!cpu.temperatures[0].hot);
    assert!(cpu.temperatures[1].hot, "past crit");
    assert!(sensors[1].temperatures[0].hot, "no crit, so max");
    assert_eq!(sensors[1].fans[0].rpm, 0);
    assert!(
        sensor_line(cpu).contains("101.0 °C (hot)"),
        "{}",
        sensor_line(cpu)
    );
}

#[test]
fn st_12_batteries_report_charge_status_and_health() {
    let dir = tempfile::TempDir::new().unwrap();
    let bat = dir.path().join("BAT0");
    write(&bat.join("type"), "Battery\n");
    write(&bat.join("capacity"), "85\n");
    write(&bat.join("status"), "Not charging\n");
    write(&bat.join("charge_full"), "4500000\n");
    write(&bat.join("charge_full_design"), "5000000\n");
    write(&dir.path().join("AC/type"), "Mains\n");

    let batteries = batteries(dir.path());

    assert_eq!(batteries.len(), 1);
    assert_eq!(batteries[0].name, "BAT0");
    assert_eq!(batteries[0].charge, Some(85));
    assert_eq!(batteries[0].status.as_deref(), Some("Not charging"));
    assert_eq!(batteries[0].health.map(|h| h.round()), Some(90.0));
}

fn healthy() -> Status {
    Status {
        cpu: Cpu {
            count: Some(4),
            load: Some([1.0, 1.0, 1.0]),
            busy: None,
            uptime: None,
        },
        memory: Some(Memory {
            total: 100,
            available: 50,
            used: 50,
            swap_total: 0,
            swap_used: 0,
        }),
        disks: vec![Disk::from_counts(
            "/".to_owned(),
            "ext4".to_owned(),
            1,
            100,
            50,
            50,
        )],
        processes: Processes::default(),
        disk_io: Vec::new(),
        network: Vec::new(),
        sensors: Vec::new(),
        batteries: Vec::new(),
        health: Health::default(),
    }
}

#[test]
fn st_13_st_14_a_healthy_machine_scores_100_and_missing_parts_cost_nothing() {
    let health = health(&healthy());
    assert_eq!(health.score, 100);
    assert!(health.reasons.is_empty());
}

#[test]
fn st_13_every_deduction_is_counted_and_explained() {
    let mut status = healthy();
    status.disks = vec![
        Disk::from_counts("/".to_owned(), "ext4".to_owned(), 1, 100, 5, 5),
        Disk::from_counts("/data".to_owned(), "ext4".to_owned(), 1, 100, 15, 15),
    ];
    status.memory = Some(Memory {
        total: 100,
        available: 5,
        used: 95,
        swap_total: 100,
        swap_used: 60,
    });
    status.cpu.load = Some([9.0, 1.0, 1.0]);
    status.processes.zombies = (0..7)
        .map(|pid| Zombie {
            pid,
            name: "z".to_owned(),
            parent: 1,
        })
        .collect();
    status.sensors = vec![Sensor {
        name: "coretemp".to_owned(),
        temperatures: vec![Temperature {
            label: None,
            celsius: 101.0,
            hot: true,
        }],
        fans: Vec::new(),
    }];
    status.batteries = vec![Battery {
        name: "BAT0".to_owned(),
        charge: Some(50),
        status: None,
        health: Some(55.0),
    }];

    let health = health(&status);

    let points: Vec<i32> = health.reasons.iter().map(|reason| reason.points).collect();
    // / at 95 %: −15; /data at 85 %: −5; memory −15; swap −5; load −10; hot −15;
    // 7 zombies, capped: −10; battery −10.
    assert_eq!(points, [-15, -5, -15, -5, -10, -15, -10, -10]);
    assert_eq!(health.score, 15);
    assert!(
        health.reasons[0].why.contains("/ is 95% full"),
        "{:?}",
        health.reasons
    );
}

#[test]
fn st_13_the_score_never_drops_below_0() {
    let mut status = healthy();
    status.disks = (0..8)
        .map(|i| Disk::from_counts(format!("/d{i}"), "ext4".to_owned(), 1, 100, 1, 1))
        .collect();
    assert_eq!(health(&status).score, 0);
}

#[test]
fn st_09_st_13_the_text_hides_idle_interfaces_and_shows_the_score() {
    let mut status = healthy();
    status.network = vec![
        Rate {
            name: "wlan0".to_owned(),
            first: 2048,
            second: 0,
        },
        Rate {
            name: "veth1".to_owned(),
            first: 0,
            second: 0,
        },
    ];
    status.health = health(&status);
    let text = status.to_string();
    assert!(text.contains("wlan0"), "{text}");
    assert!(!text.contains("veth1"), "{text}");
    assert!(
        text.contains("1 idle network interfaces not shown"),
        "{text}"
    );
    assert!(text.contains("Health 100/100"), "{text}");
}
