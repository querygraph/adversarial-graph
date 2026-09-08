//! Resource accounting so a number is never just wall-clock on a shared host:
//! client CPU (user + system) and peak RSS from `getrusage`, the host load
//! average, and — for containerized backends — the server's cumulative CPU
//! and current memory read from the Docker Engine API.

use std::process::Command;

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct Rusage {
    pub user_us: u64,
    pub sys_us: u64,
    pub maxrss_bytes: u64,
}

pub fn rusage_self() -> Rusage {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if rc != 0 {
        return Rusage::default();
    }
    let tv = |t: libc::timeval| (t.tv_sec as u64) * 1_000_000 + t.tv_usec as u64;
    // ru_maxrss is bytes on macOS and kilobytes on Linux.
    let maxrss = usage.ru_maxrss as u64 * if cfg!(target_os = "linux") { 1024 } else { 1 };
    Rusage {
        user_us: tv(usage.ru_utime),
        sys_us: tv(usage.ru_stime),
        maxrss_bytes: maxrss,
    }
}

pub fn loadavg_1m() -> f64 {
    let mut loads = [0f64; 3];
    let n = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    if n >= 1 { loads[0] } else { -1.0 }
}

/// Cumulative CPU time the hypervisor withheld from this guest, summed over
/// every vCPU, in microseconds: the `steal` column of the `cpu` line in
/// `/proc/stat`. A burstable instance out of CPU credits shows it while the
/// load average stays flat, so a run's delta is the only record of a
/// throttled measurement. `None` where the kernel does not report it.
pub fn host_cpu_steal_us() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let steal_ticks: u64 = line.split_whitespace().nth(8)?.parse().ok()?;
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    Some(steal_ticks * 1_000_000 / ticks_per_second as u64)
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ContainerUsage {
    pub cpu_usec: u64,
    pub memory_bytes: u64,
}

/// Cumulative CPU and current memory of a container from the Docker
/// Engine API (`GET /containers/{name}/stats?stream=false`), which works for
/// distroless images that have no shell for `docker exec`. Zero when the
/// daemon is unreachable.
pub fn container_usage(name: &str) -> ContainerUsage {
    let sock = std::env::var("DOCKER_SOCK").ok().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        let desktop = format!("{home}/.docker/run/docker.sock");
        if std::path::Path::new(&desktop).exists() {
            desktop
        } else {
            "/var/run/docker.sock".to_string()
        }
    });
    let out = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            "10",
            "--unix-socket",
            &sock,
            &format!("http://localhost/containers/{name}/stats?stream=false&one-shot=true"),
        ])
        .output();
    let Ok(out) = out else {
        return ContainerUsage::default();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return ContainerUsage::default();
    };
    let cpu_ns = json["cpu_stats"]["cpu_usage"]["total_usage"]
        .as_u64()
        .unwrap_or(0);
    let memory_bytes = json["memory_stats"]["usage"].as_u64().unwrap_or(0);
    ContainerUsage {
        cpu_usec: cpu_ns / 1_000,
        memory_bytes,
    }
}

/// Snapshot taken before a scenario; `finish` turns it into observations.
pub struct Probe {
    started: std::time::Instant,
    rusage: Rusage,
    load: f64,
    steal_us: Option<u64>,
    container: Option<(String, ContainerUsage)>,
}

impl Probe {
    pub fn start(container: Option<&str>) -> Self {
        Self {
            started: std::time::Instant::now(),
            rusage: rusage_self(),
            load: loadavg_1m(),
            steal_us: host_cpu_steal_us(),
            container: container.map(|c| (c.to_string(), container_usage(c))),
        }
    }

    pub fn finish(self, result: &mut crate::report::ScenarioResult) {
        let now = rusage_self();
        let wall_us = self.started.elapsed().as_micros() as u64;
        let user = now.user_us.saturating_sub(self.rusage.user_us);
        let sys = now.sys_us.saturating_sub(self.rusage.sys_us);
        result.observe("wall_us", wall_us);
        result.observe("client_user_us", user);
        result.observe("client_sys_us", sys);
        result.observe(
            "client_cpu_ratio",
            if wall_us > 0 {
                (user + sys) as f64 / wall_us as f64
            } else {
                0.0
            },
        );
        result.observe("client_maxrss_bytes", now.maxrss_bytes);
        result.observe("host_loadavg_1m_start", self.load);
        result.observe("host_loadavg_1m_end", loadavg_1m());
        if let (Some(before), Some(after)) = (self.steal_us, host_cpu_steal_us()) {
            // Summed over every vCPU; compare with wall_us times the vCPU count.
            result.observe("host_steal_us", after.saturating_sub(before));
        }
        if let Some((name, before)) = self.container {
            let after = container_usage(&name);
            result.observe("server_container", name);
            result.observe(
                "server_cpu_us",
                after.cpu_usec.saturating_sub(before.cpu_usec),
            );
            result.observe("server_memory_bytes", after.memory_bytes);
        }
    }
}

/// The process's resident set right now, from `/proc/self/status` (Linux),
/// as opposed to `ru_maxrss`, which is a high-water mark. `None` elsewhere.
pub fn current_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}
