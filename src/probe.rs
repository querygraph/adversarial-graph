//! Resource accounting so a number is never just wall-clock on a shared host:
//! client CPU (user + system) and peak RSS from `getrusage`, the host load
//! average, and — for containerized backends — the server's cumulative CPU
//! and current memory read from its cgroup through `docker exec`.

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
    Rusage { user_us: tv(usage.ru_utime), sys_us: tv(usage.ru_stime), maxrss_bytes: maxrss }
}

pub fn loadavg_1m() -> f64 {
    let mut loads = [0f64; 3];
    let n = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    if n >= 1 { loads[0] } else { -1.0 }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ContainerUsage {
    pub cpu_usec: u64,
    pub memory_bytes: u64,
}

/// Cumulative CPU (`usage_usec` from cgroup v2 `cpu.stat`) and current memory
/// (`memory.current`) of a container; zero if the container is not
/// reachable or is not cgroup v2.
pub fn container_usage(name: &str) -> ContainerUsage {
    let read = |path: &str| -> Option<String> {
        let out = Command::new("docker")
            .args(["exec", name, "cat", path])
            .output()
            .ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
    };
    let cpu_usec = read("/sys/fs/cgroup/cpu.stat")
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("usage_usec ").and_then(|v| v.trim().parse().ok()))
        })
        .unwrap_or(0);
    let memory_bytes = read("/sys/fs/cgroup/memory.current")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    ContainerUsage { cpu_usec, memory_bytes }
}

/// Snapshot taken before a scenario; `finish` turns it into observations.
pub struct Probe {
    started: std::time::Instant,
    rusage: Rusage,
    load: f64,
    container: Option<(String, ContainerUsage)>,
}

impl Probe {
    pub fn start(container: Option<&str>) -> Self {
        Self {
            started: std::time::Instant::now(),
            rusage: rusage_self(),
            load: loadavg_1m(),
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
            if wall_us > 0 { (user + sys) as f64 / wall_us as f64 } else { 0.0 },
        );
        result.observe("client_maxrss_bytes", now.maxrss_bytes);
        result.observe("host_loadavg_1m_start", self.load);
        result.observe("host_loadavg_1m_end", loadavg_1m());
        if let Some((name, before)) = self.container {
            let after = container_usage(&name);
            result.observe("server_container", name);
            result.observe("server_cpu_us", after.cpu_usec.saturating_sub(before.cpu_usec));
            result.observe("server_memory_bytes", after.memory_bytes);
        }
    }
}
