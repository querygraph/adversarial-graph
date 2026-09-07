//! A12 — cold start and footprint. Three measurements a store owner cares
//! about and a benchmark rarely reports: the time from a fresh handle (a
//! reopen of the durable state, or a new connection to a running service)
//! to the first correct answer; the resident footprint after the load; and
//! the tail of an open-loop read stream at fixed arrival rates, where
//! response time includes queueing and service time does not. No hard
//! gate is specific to this family: a wrong answer during the stream still
//! counts as `wrong_answer`, and a stream that cannot keep up is reported as
//! late arrivals, not hidden. Layers: L0, L1, L2.

use std::sync::Arc;
use std::time::{Duration, Instant};

use grust::NodeId;
use hdrhistogram::Histogram;
use tokio::sync::Semaphore;

use super::Ctx;
use crate::report::{Latency, ScenarioResult, histogram, record};

/// Arrival rates in requests per second; each runs for `duration`.
const RATES: [u64; 2] = [50, 200];
/// Concurrent handles serving the stream (each backend serializes per handle
/// to a different degree; the pool bounds in-flight work, not the arrivals).
const HANDLES: usize = 16;

fn quantiles(h: &Histogram<u64>) -> (u64, u64, u64, u64) {
    (
        h.value_at_quantile(0.5),
        h.value_at_quantile(0.99),
        h.value_at_quantile(0.999),
        h.max(),
    )
}

pub async fn run(ctx: &Ctx<'_>) -> ScenarioResult {
    let mut r = ScenarioResult::new("A12", ctx.backend.kind.name(), ctx.dataset);
    let (hub, oracle_degree) = ctx.oracle.max_out_degree_vertex();
    // A4 runs first in a full ladder and appends to the same hub; those
    // durable edges are part of the correct answer from here on.
    let appended = ctx.hub_writes.load(std::sync::atomic::Ordering::SeqCst);
    let hub_degree = oracle_degree + appended;
    r.observe("hub", hub.as_str());
    r.observe("hub_edges_appended_by_a4", appended);

    // 1. Cold start: a fresh handle and the first correct hub degree.
    let t = Instant::now();
    match ctx.backend.out_degree_after_reopen(&hub).await {
        Ok(d) if d == hub_degree => {
            r.observe("cold_start_ms", t.elapsed().as_millis());
        }
        Ok(d) => {
            r.gates.wrong_answer += 1;
            r.notes
                .push(format!("cold-start degree {d} != oracle {hub_degree}"));
        }
        Err(e) if crate::backends::Backend::is_unsupported(&e) => {
            r.unsupported(&format!("backend cannot read edges back: {e}"));
            return r;
        }
        Err(e) => {
            r.gates.oom_or_crash += 1;
            r.notes.push(format!("cold start failed: {e}"));
            return r;
        }
    }

    // 2. Footprint: the client's peak resident set so far (after the load)
    // is in the probe's client_maxrss_bytes; the service's memory is
    // server_memory_bytes. Both land on this row through the probe.

    // 3. Open-loop stream of one-hop reads at fixed arrival rates over a
    // fixed vertex sample (every 50th request hits the hub).
    let sample: Vec<NodeId> = ctx.oracle.sample_vertices(1_000);
    if sample.is_empty() {
        r.unsupported("dataset has no vertices to sample");
        return r;
    }
    let duration = Duration::from_secs(if ctx.smoke { 10 } else { 30 });
    let mut handles: Vec<Arc<dyn grust::GraphStore>> = Vec::with_capacity(HANDLES);
    for _ in 0..HANDLES {
        match ctx.backend.extra_handle().await {
            Ok(h) => handles.push(h),
            Err(e) => {
                r.gates.oom_or_crash += 1;
                r.notes.push(format!("could not open stream handle: {e}"));
                return r;
            }
        }
    }
    let handles = Arc::new(handles);
    let mut merged_service = histogram();
    for rate in RATES {
        let interval = Duration::from_nanos(1_000_000_000 / rate);
        let semaphore = Arc::new(Semaphore::new(HANDLES));
        let start = Instant::now();
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        let mut tasks = Vec::new();
        let mut sent = 0u64;
        let mut in_flight_max = 0usize;
        while start.elapsed() < duration {
            ticker.tick().await;
            let scheduled = Instant::now();
            let vertex = if sent % 50 == 0 {
                hub.clone()
            } else {
                sample[(sent as usize) % sample.len()].clone()
            };
            let expected = if vertex == hub {
                hub_degree
            } else {
                ctx.oracle.out_degree(&vertex)
            };
            let permit = semaphore.clone().acquire_owned().await.expect("semaphore");
            let in_flight = HANDLES - semaphore.available_permits();
            in_flight_max = in_flight_max.max(in_flight);
            let store = handles[(sent as usize) % HANDLES].clone();
            sent += 1;
            tasks.push(tokio::spawn(async move {
                let queued = scheduled.elapsed();
                let t = Instant::now();
                let got = store
                    .get_edges(grust::EdgeQuery {
                        from: Some(vertex),
                        to: None,
                        label: Some(crate::dataset::EDGE_LABEL.into()),
                    })
                    .await
                    .map(|e| e.len());
                drop(permit);
                (queued, t.elapsed(), got, expected)
            }));
        }
        let mut service = histogram();
        let mut response = histogram();
        let mut wrong = 0u64;
        let mut errors = 0u64;
        let mut late = 0u64;
        for task in tasks {
            match task.await {
                Ok((queued, took, got, expected)) => {
                    record(&mut service, took);
                    record(&mut response, queued + took);
                    if queued > Duration::from_secs(1) {
                        late += 1;
                    }
                    match got {
                        Ok(n) if n == expected => {}
                        Ok(_) => wrong += 1,
                        Err(_) => errors += 1,
                    }
                }
                Err(_) => errors += 1,
            }
        }
        let _ = merged_service.add(&service);
        let (s50, s99, s999, smax) = quantiles(&service);
        let (r50, r99, r999, rmax) = quantiles(&response);
        let prefix = format!("stream_{rate}rps");
        r.observe(&format!("{prefix}_sent"), sent);
        r.observe(
            &format!("{prefix}_achieved_rps"),
            (sent as f64 / start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        );
        r.observe(&format!("{prefix}_service_p50_us"), s50);
        r.observe(&format!("{prefix}_service_p99_us"), s99);
        r.observe(&format!("{prefix}_service_p999_us"), s999);
        r.observe(&format!("{prefix}_service_max_us"), smax);
        r.observe(&format!("{prefix}_response_p50_us"), r50);
        r.observe(&format!("{prefix}_response_p99_us"), r99);
        r.observe(&format!("{prefix}_response_p999_us"), r999);
        r.observe(&format!("{prefix}_response_max_us"), rmax);
        r.observe(&format!("{prefix}_max_in_flight"), in_flight_max);
        r.observe(&format!("{prefix}_late_arrivals_over_1s"), late);
        r.observe(&format!("{prefix}_errors"), errors);
        if wrong > 0 {
            r.gates.wrong_answer += wrong;
            r.notes.push(format!(
                "{wrong} wrong one-hop degrees in the {rate} rps stream"
            ));
        }
        if errors > 0 {
            r.notes.push(format!(
                "{errors} errored requests in the {rate} rps stream"
            ));
        }
    }
    r.latency = Some(Latency::from_histogram(&merged_service));
    r
}
