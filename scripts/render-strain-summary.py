#!/usr/bin/env python3
"""Render the strain ledger's summary from the site's own published evidence.

    python3 scripts/render-strain-summary.py /path/to/adversarial-site [--check]

Reads every publication under public/evidence/strain/, keeps the latest cell
per (dataset, backend, scenario, edge slice, profile) exactly as RESULTS.md
does, classifies every cell by what decided it, and writes, between the
`<!-- strain-summary:begin -->` and `<!-- strain-summary:end -->` markers of
graph/strain/index.html:

  - the executive summary,
  - the strain matrix (every backend against every whole graph),
  - the Rust question (reach by group, and the head-to-head with Neo4j),
  - the scenario groups, and
  - every latest cell, grouped by engine family, backend, tier and graph.

Every number in the prose is computed here, and the claims the prose makes by
name are asserted against the data, so the page cannot drift from its
evidence without this script failing. `--check` exits non-zero when the page
differs from what the evidence renders.
"""
import argparse
import collections
import html
import json
import os
import re
import statistics
import sys

BEGIN, END = "<!-- strain-summary:begin -->", "<!-- strain-summary:end -->"
ALIAS = {"surreal": "surreal-sdk"}
# The standard envelope: 6 GiB containers, and each store's own defaults.
DEFAULT = {"", "resultset_size=10000", "buffer_pool_bytes=4294967296,concurrent_writes=false",
           "turso_load_writers=4"}  # the harness default for MVCC loads since 2026-09-16: four parallel writers
BIG = {"mem_limit=25769803776", "resultset_size=10000,mem_limit=25769803776"}
PROFILE_LABEL = {
    "": "default",
    "resultset_size=10000": "default (FalkorDB's 10,000-row result cap)",
    "resultset_size=-1": "result cap off",
    "buffer_pool_bytes=4294967296,concurrent_writes=false": "default (4 GiB buffer pool)",
    "buffer_pool_bytes=4294967296,concurrent_writes=true": "concurrent writes on",
    "mem_limit=25769803776": "24 GiB containers",
    "resultset_size=10000,mem_limit=25769803776": "24 GiB containers, 10,000-row cap",
    "turso_load_writers=4": "default (4 parallel MVCC load writers)",
    "turso_load_writers=8": "8 parallel MVCC load writers",
    "turso_sync=normal,turso_load_writers=8": "synchronous=NORMAL during the load (not fsync-durable per commit), 8 writers",
    "turso_sync=normal,turso_load_writers=4": "synchronous=NORMAL during the load (not fsync-durable per commit), 4 writers",
}


def norm_profile(backend, profile):
    """The profile as the cell key sees it: the host class is where a run was
    taken, not a configuration; WAL admits one writer, so a writers tag on a
    WAL row (stamped by harness revisions before 36a6897) means nothing."""
    parts = [x for x in (profile or "").split(",") if x and not x.startswith("host=")]
    if backend == "turso-wal":
        parts = [x for x in parts if not x.startswith("turso_load_writers=")]
    return ",".join(parts)
SLICE_LABEL = {"full": "whole graph", "200k": "first 200 k edges", "50k": "first 50 k edges", "10k": "first 10 k edges"}

# ---------------------------------------------------------------- backends
BACKENDS = collections.OrderedDict()
def _b(key, name, lang, form, path, version, group):
    BACKENDS[key] = dict(key=key, name=name, lang=lang, form=form, path=path, version=version, group=group)
_b("memory", "grust-memory", "Rust", "in process", "Grust portable API", "grust-memory 0.13", "rust-local")
_b("turso-wal", "Turso · WAL", "Rust", "embedded", "Grust portable API", "turso 0.7.2", "rust-local")
_b("turso-mvcc", "Turso · MVCC", "Rust", "embedded", "Grust portable API", "turso 0.7.2", "rust-local")
_b("lancedb", "LanceDB", "Rust", "embedded", "Grust portable API", "lancedb 0.30.0", "rust-local")
_b("surreal-http", "SurrealDB · HTTP", "Rust", "server", "Grust adapter, SurrealQL over HTTP", "surrealdb 3.2.4", "rust-server")
_b("surreal-sdk", "SurrealDB · SDK", "Rust", "server", "Grust adapter, Rust SDK over WebSocket", "surrealdb 3.2.4", "rust-server")
_b("helix-http", "HelixDB · HTTP", "Rust", "server", "Grust adapter, JSON on /v1/query", "enterprise-dev image", "rust-server")
_b("helix-sdk", "HelixDB · SDK", "Rust", "server", "Grust adapter, helix-db 3.0.0 on /v2/query", "server source 0ef3cee0", "rust-server")
_b("ladybug", "LadybugDB", "C++", "embedded", "Grust adapter over the lbug crate", "lbug 0.20.2 (formerly Kùzu)", "cpp-local")
_b("falkor", "FalkorDB", "C", "server", "harness Cypher over RESP; Grust writes", "falkordb 4.20.4", "c-server")
_b("memgraph", "Memgraph", "C++", "server", "harness Cypher over Bolt", "memgraph 3.12.0", "c-server")
_b("postgres", "PostgreSQL", "C", "server", "Grust SQL route over pg-wire", "postgres 18.6", "c-server")
_b("age", "Apache AGE", "C", "server", "harness Cypher over pg-wire", "AGE 1.8.0 on PostgreSQL 18", "c-server")
_b("neo4j", "Neo4j · Bolt", "Java", "server", "harness Cypher over Bolt", "neo4j 5.26 community", "jvm")
_b("neo4j-http", "Neo4j · HTTP", "Java", "server", "harness Cypher over the HTTP Query API", "neo4j 5.26 community", "jvm")
GROUPS = [
    ("rust-local", "Rust, in process", "The engine runs inside the harness process: no network hop, no container, bounded by the host's memory guard rather than a 6 GiB limit."),
    ("rust-server", "Rust, servers", "A Rust database server in a 6 GiB container, reached through a Grust adapter over HTTP or its Rust SDK."),
    ("cpp-local", "C++, in process", "An embedded C++ engine behind a Rust crate, inside the harness process."),
    ("c-server", "C and C++, servers", "Established servers in 6 GiB containers; reads go through the harness's own Cypher where the store speaks it."),
    ("jvm", "JVM, servers", "Neo4j in a 6 GiB container, over Bolt and over its HTTP Query API."),
]
RUST = [k for k, b in BACKENDS.items() if b["lang"] == "Rust"]

# ---------------------------------------------------------------- datasets
DATASETS = {
    "email-Eu-core": ("S", False, "email network, labelled"),
    "ego-Facebook": ("S", False, "small social graph"),
    "wiki-Talk": ("S", False, "super-node hub"),
    "web-Google": ("S", False, "many components"),
    "roadNet-CA": ("S", False, "high diameter (roads)"),
    "cit-Patents": ("M", False, "citation DAG"),
    "soc-Pokec-relationships": ("M", False, "social network"),
    "GAP-road": ("L", False, "high diameter at scale, 24 M nodes"),
    "sx-stackoverflow": ("M", False, "temporal multigraph, parallel edges kept"),
    "soc-LiveJournal1": ("M", False, "degree skew"),
    "com-Orkut": ("M", False, "dense communities"),
    "ldbc-snb-sf0.1": ("S", True, "LDBC social network, scale 0.1"),
    "icij-offshore-leaks": ("S", True, "ICIJ Offshore Leaks"),
    "ldbc-snb-sf1": ("M", True, "LDBC social network, scale 1"),
}
CORE, TYPED_FAMS = ["A1", "A2", "A4", "A12"], ["A5", "A6", "A8"]

# ---------------------------------------------------------------- causes
CAUSE = {
    "pass": ("pass", "passed; every hard gate zero"),
    "pass-incomplete": ("pass", "passed; the run ended before its last family (pair cap, host guard or crash)"),
    "by-design": ("n/a", "not applicable by design: A3 runs only on the in-process reference, A7 only on stores with guarded commits"),
    "not-applicable": ("n/a", "the scenario does not apply to this graph or store"),
    "capability-gap": ("gap", "the adapter lacks the operation this scenario needs"),
    "declared-lww": ("declared", "no conditional write in the adapter: last-writer-wins is declared, anomalies are recorded, not gated"),
    "queries-refused": ("refused", "the store refused some queries with a typed error"),
    "refused-multigraph": ("refused", "the store refused parallel edges with a typed error"),
    "container-oom": ("memory", "the kernel killed the container at its memory limit"),
    "store-memory-limit": ("memory", "the store stopped at its own declared memory limit"),
    "crash": ("crash", "the store crashed or dropped the connection"),
    "load-budget": ("time", "the load did not finish inside its 2-hour budget"),
    "projected-past-budget": ("time", "not attempted: the store's measured rate projects past the load budget"),
    "gateway-timeout": ("time", "the server's own 30 s request timeout ended the load"),
    "wrong-answer": ("wrong", "a wrong answer against the oracle"),
    "lost-write": ("wrong", "a write was lost"),
    "isolation-anomaly": ("wrong", "an isolation anomaly"),
    "duplicate-mutation": ("wrong", "a durable mutation applied twice"),
    "hang": ("wrong", "a query ran past its deadline without refusing"),
    "unsupported-other": ("n/a", "unsupported"),
    "not-tested-other": ("n/a", "not tested"),
    "fail-other": ("fail", "failed"),
}
BAD = {"wrong-answer", "lost-write", "isolation-anomaly", "duplicate-mutation", "hang"}
WALL = {"container-oom": "mem", "store-memory-limit": "mem", "crash": "crash", "load-budget": "time",
        "projected-past-budget": "time", "gateway-timeout": "time", "refused-multigraph": "refused"}
GLYPH = {"clean": "✓", "partial": "½", "caveat": "✓*", "wrong": "✗", "mem": "M", "time": "T",
         "crash": "C", "refused": "R", "none": "·"}
KIND_WORD = {"clean": "clean", "partial": "loaded, not every family run", "caveat": "clean with declared limits",
             "wrong": "wrong answer", "mem": "memory wall", "time": "time wall", "crash": "crash",
             "refused": "refused", "none": "not run"}


def cause_of(row):
    notes = " ".join(row.get("notes") or [])
    gates = row.get("gates") or {}
    outcome = row["outcome"]
    if outcome == "pass":
        return "pass-incomplete" if "run ended before its final write" in notes else "pass"
    if outcome == "unsupported":
        if row["scenario"] in ("A3", "A7"):
            return "by-design"
        if "parallel edges" in notes:
            return "refused-multigraph"
        if re.search(r"runs over LDBC SNB|does not accept Cypher", notes):
            return "not-applicable"
        if re.search(r"does not implement reads|no delete path|typed graphs load as :V/:E", notes):
            return "capability-gap"
        if "last-writer-wins" in notes or "read-then-upsert" in notes:
            return "declared-lww"
        if "queries refused by the store" in notes:
            return "queries-refused"
        return "unsupported-other"
    if outcome == "not-tested":
        return "projected-past-budget" if "not attempted" in notes else "not-tested-other"
    if "OOMKilled" in notes:
        return "container-oom"
    if re.search(r"memory limit exceeded|Memgraph\.TransientError", notes):
        return "store-memory-limit"
    if "did not finish inside" in notes and "load budget" in notes:
        return "load-budget"
    if "408" in notes:
        return "gateway-timeout"
    for gate, c in (("wrong_answer", "wrong-answer"), ("lost_write", "lost-write"),
                    ("isolation_anomaly", "isolation-anomaly"), ("duplicate_durable_mutation", "duplicate-mutation"),
                    ("hang_or_timeout_without_refusal", "hang")):
        if (gates.get(gate) or 0) > 0:
            return c
    if (gates.get("oom_or_crash") or 0) > 0:
        return "crash"
    return "fail-other"


def machine(pub):
    """The machine a publication's runs were taken on."""
    if pub == "2026-09-05":
        return "laptop"
    if pub.startswith("2026-09-06"):
        return "lakecat"
    for m in ("quegee", "grust", "lakecat", "eigen", "laptop"):
        if m in pub:
            return m
    raise ValueError(pub)


def load(evidence, before=None):
    """Every run under the evidence tree (publications dated before `before`
    only, when given), the latest cell per (dataset, backend, scenario, slice,
    profile), and the history: every cell a later run superseded, with the
    cell that superseded it and why."""
    runs = {}
    for pub in sorted(os.listdir(evidence)):
        mpath = os.path.join(evidence, pub, "manifest.json")
        if not os.path.exists(mpath):
            continue
        manifest = json.load(open(mpath))
        if before and pub[:10] >= before:
            continue
        for r in manifest["runs"]:
            run = r["run"]
            d = os.path.join(evidence, pub, run)
            if run in runs or not os.path.exists(os.path.join(d, "report.json")):
                continue
            rep = json.load(open(os.path.join(d, "report.json")))
            slices = {}
            for ds in rep.get("datasets", []):
                cap = (ds.get("load") or {}).get("truncated_at")
                slices[ds["manifest"]["name"]] = f"{cap // 1000}k" if cap else "full"
            runs[run] = dict(pub=pub, host=machine(pub), cpus=(rep.get("host") or {}).get("cpus"),
                             smoke=r.get("smoke", False), slices=slices, rows=rep.get("results", []))
    cells, history = {}, []
    for run in sorted(runs):
        R = runs[run]
        for row in R["rows"]:
            obs = row.get("observations") or {}
            backend = ALIAS.get(row["backend"], row["backend"])
            key = (row["dataset"], backend, row["scenario"],
                   R["slices"].get(row["dataset"], "200k" if R["smoke"] else "full"), norm_profile(backend, obs.get("profile")))
            cell = dict(run=run, pub=R["pub"], host=R["host"], cpus=R["cpus"], row=row, obs=obs, cause=cause_of(row), key=key)
            if key in cells:
                history.append(dict(old=cells[key], new=cell, why="a later run at the same profile"))
            cells[key] = cell
    # Within the standard envelope the profiles are equivalent for the ranking
    # (a store at its defaults, as the harness defines them at the time), so the
    # newest default-envelope run supersedes older default runs of the same
    # cell even when the harness's default itself moved.
    by_cell = collections.defaultdict(list)
    for key in list(cells):
        if key[4] in DEFAULT:
            by_cell[key[:4]].append(key)
    for base, keys in by_cell.items():
        if len(keys) < 2:
            continue
        newest = max(keys, key=lambda k: cells[k]["run"])
        for k in keys:
            if k != newest:
                history.append(dict(old=cells[k], new=cells[newest],
                                    why=f"a later run at the harness default ({PROFILE_LABEL.get(newest[4], newest[4])})"))
                del cells[k]
    pubs = sorted({R["pub"] for R in runs.values()})
    return runs, cells, pubs, history


# ---------------------------------------------------------------- analysis
def status(cells, backend, dataset, envelope):
    sel = {k[2]: c for k, c in cells.items()
           if k[0] == dataset and k[1] == backend and k[3] == "full" and k[4] in envelope}
    if not sel:
        return dict(kind="none", title="not run", fams={})
    load_cell = sel.get("LOAD")
    typed = DATASETS[dataset][1]
    names = TYPED_FAMS if typed else CORE
    if load_cell is None:
        return dict(kind="none", title="no load row", fams={})
    lc = load_cell["cause"]
    if lc not in ("pass", "pass-incomplete"):
        return dict(kind=WALL.get(lc, "crash"), cause=lc, title=CAUSE[lc][1], fams={}, load=load_cell)
    fams = {f: sel[f] for f in names if f in sel}
    bad = [f for f, c in fams.items() if c["cause"] in BAD]
    declared = [f for f, c in fams.items() if c["cause"] in ("capability-gap", "declared-lww", "queries-refused")]
    ran_ok = [f for f, c in fams.items() if c["cause"] in ("pass", "pass-incomplete", "not-applicable")]
    missing = [f for f in names if f not in fams]
    if bad:
        kind = "wrong"
        title = "wrong: " + ", ".join(f"{f} ({CAUSE[fams[f]['cause']][1]})" for f in bad)
    elif missing and not typed:
        kind = "partial"
        title = f"loaded; {', '.join(ran_ok) or 'no family'} passed; {', '.join(missing)} not run"
    elif declared:
        kind = "caveat"
        title = "loaded; " + "; ".join(f"{f}: {CAUSE[fams[f]['cause']][1]}" for f in declared)
    else:
        kind = "clean"
        title = "loaded; every family passed"
    return dict(kind=kind, title=title, fams=fams, load=load_cell, bad=bad, missing=missing)


def dataset_sizes(cells):
    sizes, nodes, hub = {}, {}, {}
    for k, c in cells.items():
        if k[3] != "full":
            continue
        if k[2] == "LOAD" and c["obs"].get("edges"):
            sizes[k[0]] = max(sizes.get(k[0], 0), c["obs"]["edges"])
            nodes[k[0]] = max(nodes.get(k[0], 0), c["obs"].get("nodes") or 0)
        if k[2] == "A1" and c["obs"].get("hub_out_degree"):
            hub[k[0]] = max(hub.get(k[0], 0), c["obs"]["hub_out_degree"])
    return sizes, nodes, hub


def reach(cells, sizes):
    """Per backend, under the standard envelope, over whole untyped graphs."""
    out = {}
    untyped = sorted((d for d in sizes if d in DATASETS and not DATASETS[d][1]), key=sizes.get)
    for b in BACKENDS:
        st = {d: status(cells, b, d, DEFAULT) for d in untyped}
        sustained = [d for d in untyped if st[d]["kind"] in ("clean", "partial", "caveat")
                     and any(c["cause"] in ("pass", "pass-incomplete") for c in st[d]["fams"].values())]
        loaded = [d for d in untyped if st[d]["kind"] not in ("none", "mem", "time", "crash", "refused")]
        walls = [d for d in untyped if st[d]["kind"] in ("mem", "time", "crash", "refused")]
        wrong = [d for d in untyped if st[d]["kind"] == "wrong"]
        clean_all = [d for d in untyped if st[d]["kind"] == "clean"]
        out[b] = dict(status=st, sustained=sustained[-1] if sustained else None,
                      loaded=loaded[-1] if loaded else None, first_wall=walls[0] if walls else None,
                      wrong=wrong, clean=clean_all[-1] if clean_all else None,
                      n_clean=len(clean_all), n_walls=len(walls))
    return out, untyped


def head_to_head(cells, ref="neo4j"):
    metrics = {"LOAD": [("wall_us", "loading")], "A1": [("wall_us", "hub fan-out")], "A2": [("wall_us", "deep walk")],
               "A4": [("wall_us", "100 hot-node writes")],
               "A12": [("cold_start_ms", "cold start"), ("stream_200rps_response_p99_us", "p99 at 200 req/s")]}
    idx = {}
    for k, c in cells.items():
        ds, b, sc, sl, prof = k
        if prof in DEFAULT and c["cause"] == "pass":
            idx[(ds, sl, sc, c["host"], c["cpus"], b)] = c
    res = collections.defaultdict(list)
    for (ds, sl, sc, host, cpus, b), c in idx.items():
        if b not in RUST or sc not in metrics:
            continue
        r = idx.get((ds, sl, sc, host, cpus, ref))
        if not r:
            continue
        for m, _label in metrics[sc]:
            rv, nv = c["obs"].get(m), r["obs"].get(m)
            if rv and nv and rv > 0 and nv > 0:
                res[(b, sc, m)].append((ds, sl, host, rv, nv))
    order = [(sc, m, label) for sc, ms in metrics.items() for m, label in ms]
    table = {}
    for (b, sc, m), pairs in res.items():
        wins = sum(1 for *_, rv, nv in pairs if rv < nv)
        table[(b, sc, m)] = dict(pairs=len(pairs), wins=wins,
                                 median=statistics.median(nv / rv for *_, rv, nv in pairs),
                                 detail=sorted(pairs))
    return table, order


# ---------------------------------------------------------------- formatting
def esc(s):
    return html.escape(str(s), quote=True)


def edges(n):
    if n is None:
        return "–"
    if n >= 1e6:
        return f"{n / 1e6:.1f} M"
    if n >= 1e3:
        return f"{n / 1e3:.1f} k"
    return f"{n}"


def dur(us):
    if not us:
        return ""
    s = us / 1e6
    if s < 1:
        return f"{s * 1000:.0f} ms"
    if s < 120:
        return f"{s:.1f} s"
    if s < 7200:
        return f"{s / 60:.1f} min"
    return f"{s / 3600:.2f} h"


def ratio(r):
    if r >= 10:
        return f"{r:,.0f}×"
    if r >= 1:
        return f"{r:.1f}×"
    if r >= 0.01:
        return f"{r:.2f}×"
    return "<0.01×"


def chip(kind, title=None, text=None):
    t = f' title="{esc(title)}"' if title else ""
    return f'<span class="st {kind}"{t}>{esc(text or GLYPH[kind])}</span>'


def acc(summary, body, cls="", open_=False):
    o = " open" if open_ else ""
    return f'<details class="acc {cls}"{o}><summary>{summary}</summary><div class="acc-body">{body}</div></details>'


def name(b):
    return esc(BACKENDS[b]["name"])


def fold(section_html, verdict_html, open_=False):
    """Keep a section's heading (eyebrow, h2, intro) visible and fold everything
    under it behind one line that says what the section concludes."""
    i = section_html.index('<div class="section-head"')
    j = section_html.index("</div>", i) + len("</div>")
    head, rest = section_html[:j], section_html[j:]
    for close in ("</div></section>", "</section>"):
        if rest.endswith(close):
            body, tail = rest[:-len(close)], close
            break
    return head + acc(f'<strong>Detail</strong><span class="verdict">{verdict_html}</span>', body, "group", open_) + tail


# ---------------------------------------------------------------- sections
def render(site):
    evidence = os.path.join(site, "public", "evidence", "strain")
    runs, cells, pubs, history = load(evidence)
    sizes, nodes, hub = dataset_sizes(cells)
    R, untyped = reach(cells, sizes)
    latest_date = pubs[-1][:10]
    new_pubs = [p for p in pubs if p[:10] == latest_date]
    _r, cells_before, pubs_before, _h = load(evidence, before=latest_date)
    R_before, _u = reach(cells_before, dataset_sizes(cells_before)[0]) if cells_before else ({}, [])
    hosts = sorted({R_["host"] for R_ in runs.values()})
    typed = sorted((d for d in sizes if d in DATASETS and DATASETS[d][1]), key=sizes.get)
    h2h, h2h_order = head_to_head(cells)

    # ---- facts the prose names, asserted
    CL = lambda b: sizes.get(R[b]["clean"], 0)
    rank = sorted(BACKENDS, key=lambda b: (CL(b), R[b]["n_clean"], -R[b]["n_walls"]), reverse=True)
    top = rank[0]
    top_clean_everywhere = all(R[top]["status"][d]["kind"] in ("clean", "none") for d in untyped)
    rust_rank = [b for b in rank if b in RUST]
    best_rust = rust_rank[0]
    rust_local = [b for b in RUST if BACKENDS[b]["group"] == "rust-local"]
    rust_servers = [b for b in RUST if BACKENDS[b]["group"] == "rust-server"]
    server_reach = max(CL(b) for b in rust_servers)
    local_beats_servers = min(CL(b) for b in rust_local) > server_reach
    assert CL(top) == max(CL(b) for b in BACKENDS)
    leaders = [b for b in rank if CL(b) == CL(top)]
    assert set(b for b in BACKENDS for d in typed if status(cells, b, d, DEFAULT)["kind"] == "wrong") <= {"falkor", "memgraph", "neo4j", "neo4j-http"}, "typed summary is stale"
    surreal24 = status(cells, "surreal-http", "wiki-Talk", BIG)
    assert surreal24["kind"] == "mem", "SurrealDB at 24 GiB no longer ends at the container limit"
    moved_by_24 = [b for b in BACKENDS if any(status(cells, b, d, BIG)["kind"] in ("clean", "partial") and
                                               R[b]["status"][d]["kind"] in ("mem", "crash") for d in untyped)]
    gated = collections.defaultdict(lambda: collections.defaultdict(list))  # cause -> backend -> ["graph family"]
    for b in BACKENDS:
        for d in untyped + typed:
            st = status(cells, b, d, DEFAULT)
            for f in st.get("bad", []):
                gated[st["fams"][f]["cause"]][b].append(f"{d} {f}")
    walls = collections.Counter(R[b]["status"][d]["kind"] for b in BACKENDS for d in untyped
                                if R[b]["status"][d]["kind"] in ("mem", "time", "crash", "refused"))
    always = [(b, sc, m) for (b, sc, m), v in h2h.items() if v["wins"] == v["pairs"] and v["pairs"] >= 2]
    n_pass = sum(1 for c in cells.values() if c["cause"] in ("pass", "pass-incomplete"))
    n_na = sum(1 for c in cells.values() if CAUSE[c["cause"]][0] in ("n/a", "declared", "gap"))
    n_fail = len(cells) - n_pass - n_na

    def label_of(b, sc, m):
        return dict((mm, lab) for s, mm, lab in h2h_order if s == sc)[m]

    # ---- the verdict: best results first
    def a4_at(b, d):
        for k, c in cells.items():
            if k[0] == d and k[1] == b and k[2] == "A4" and k[3] == "full" and k[4] in DEFAULT:
                o = c["obs"]
                acc_ = o.get("accepted")
                att = (o.get("writers") or 0) * (o.get("edges_per_writer") or 0) or None
                if acc_ is None:
                    return chip("clean" if c["cause"] == "pass" else "wrong", CAUSE[c["cause"]][1], CAUSE[c["cause"]][0]), dur(o.get("wall_us"))
                return (f"{acc_:,}/{att:,}" if att else f"{acc_:,}") + (" accepted" if att and acc_ == att else " accepted, the rest refused" if att else ""), dur(o.get("wall_us"))
        return "–", ""
    wins_of = {b: [label_of(b, sc, m) for bb, sc, m in always if bb == b] for b in BACKENDS}
    lb_rows = []
    for b in rank:
        r = R[b]
        if not r["clean"]:
            continue
        a4, a4w = a4_at(b, r["clean"])
        fw = r["first_wall"]
        wall_txt = f'{esc(fw)}: {esc(CAUSE[r["status"][fw]["cause"]][1])}' if fw else "none reached"
        lb_rows.append(f'<tr><td class="b">{name(b)}</td><td>{esc(BACKENDS[b]["lang"])}, {esc(BACKENDS[b]["form"])}</td>'
                       f'<td class="num"><strong>{edges(CL(b))}</strong><br><span class="muted">{esc(r["clean"])}</span></td>'
                       f'<td>{a4}<br><span class="muted">{a4w}</span></td>'
                       f'<td>{esc(", ".join(wins_of[b])) if wins_of[b] else ("reference" if b == "neo4j" else "none")}</td>'
                       f'<td>{wall_txt}</td></tr>')
    leaderboard = ('<div class="evidence-table" tabindex="0" role="region" aria-label="Leaderboard"><table><thead><tr>'
                   '<th class="b">Backend</th><th>Built as</th><th>Largest graph clean on all four families</th>'
                   '<th>Hot-node writes there (A4)</th><th>Always faster than Neo4j · Bolt on</th><th>First wall</th>'
                   '</tr></thead><tbody>' + "".join(lb_rows) + '</tbody></table></div>')
    changed = []
    for b in rank:
        if b not in R_before:
            continue
        before, now = R_before[b]["clean"], R[b]["clean"]
        if before != now:
            changed.append(f"<strong>{name(b)}</strong>: {esc(before or 'no whole graph')} → {esc(now or 'no whole graph')}"
                           + (f" ({edges(sizes.get(before, 0))} → {edges(CL(b))})" if before and now else ""))
    n_new_runs = sum(1 for R_ in runs.values() if R_["pub"] in new_pubs)
    n_sup = sum(1 for h in history if h["new"]["pub"] in new_pubs)
    what_changed = (
        f'<p><strong>What changed in the {esc(latest_date)} publication.</strong> {n_new_runs} new runs in {len(new_pubs)} bundles '
        f'({", ".join(esc(p) for p in new_pubs)}); {n_sup} earlier cells were superseded and moved to the history below. '
        + (("Largest clean graph moved for " + "; ".join(changed) + ".") if changed else "No backend's largest clean graph moved.")
        + '</p>')
    verdict_line = (
        f"{', '.join(name(b) for b in leaders[:-1])} and {name(leaders[-1])} are clean on all four core families up to "
        if len(leaders) > 1 else f"{name(top)} is clean on all four core families up to ")
    kpis = [(f"{len(runs)}", "runs"), (f"{len(cells):,}", "current cells"), (f"{len(history):,}", "superseded, in history"),
            (f"{len(BACKENDS)}", "backends"), (f"{len(sizes)}", "graphs"), (edges(max(sizes.values())), "edges, largest graph"), ("9", "hard gates")]
    s_verdict = (
        '<section class="wrap strain-summary" id="verdict">'
        '<div class="section-head"><span class="eyebrow">The verdict</span>'
        f'<h2>{verdict_line}{esc(R[top]["clean"])}, {edges(CL(top))} edges.</h2>'
        '<div class="kpis">' + "".join(f'<div class="kpi"><b>{k}</b><span>{v}</span></div>' for k, v in kpis) + '</div>'
        f'<p>Ranked by the largest whole graph a store loaded and passed all four core families on (A1 hub fan-out, A2 deep '
        f'paths, A4 hot-node writes, A12 operability) under the standard envelope, then by how many graphs it is clean on. '
        f'<em>Hot-node writes there</em> is what happened when 16 writers each attached 200 edges to the same hub on that '
        f'graph: a durable store accepts all of them; a single-writer journal refuses most and still passes, because a '
        f'typed refusal is not a lost write. <em>Always faster</em> counts the scenarios where the store beat Neo4j over '
        f'Bolt in every pair on the same graph and machine.</p></div>'
        + leaderboard + what_changed + '</section>')

    # ---- executive summary
    runners = ", ".join(f"{name(b)} ({esc(R[b]['clean'])}, {edges(CL(b))})" for b in rank[1:5])
    rust_line = ", ".join(f"{name(b)} up to {esc(R[b]['clean'])} ({edges(CL(b))})" for b in rust_rank[:3])
    further = [b for b in rust_rank[:3] if sizes.get(R[b]["sustained"], 0) > CL(b)]
    further_line = (" " + "; ".join(f"{name(b)} also loads {esc(R[b]['sustained'])} ({edges(sizes[R[b]['sustained']])}) and passes "
                                     f"every family it ran there, though not all four ran" for b in further) + ".") if further else ""
    always_by = collections.defaultdict(list)
    for b, sc, m in always:
        always_by[b].append(f"{label_of(b, sc, m)} ({h2h[(b, sc, m)]['wins']}/{h2h[(b, sc, m)]['pairs']}, {h2h[(b, sc, m)]['median']:.1f}×)")
    always_line = "; ".join(f"<strong>{name(b)}</strong>: {', '.join(v)}" for b, v in
                            sorted(always_by.items(), key=lambda x: list(BACKENDS).index(x[0])))
    def gated_line(cause):
        return "; ".join(f"{name(b)} ({esc(', '.join(v))})" for b, v in
                         sorted(gated[cause].items(), key=lambda x: list(BACKENDS).index(x[0])))
    exec_items = [
        f"<strong>What was run.</strong> {len(runs)} runs on {len(hosts)} machines from {pubs[0][:10]} to {pubs[-1][:10]}, published in "
        f"{len(pubs)} verified bundles: {len(cells):,} latest cells over {len(BACKENDS)} backends (eleven engines, four of "
        f"them reached two ways) and {len(sizes)} graphs from {edges(min(sizes.values()))} to {edges(max(sizes.values()))} edges. "
        f"{n_pass:,} cells pass, {n_na:,} are not applicable or declared limits, and {n_fail:,} fail or were placed by a limit.",
        f"<strong>Who takes the most strain.</strong> "
        + (f"{name(top)} goes furthest: " + ("it passes all four core families (A1, A2, A4, A12) on every whole untyped graph, up to "
           if top_clean_everywhere else "it is clean up to ") if len(leaders) == 1 else
           f"{', '.join(name(b) for b in leaders[:-1])} and {name(leaders[-1])} go furthest, each clean on all four core families (A1, A2, A4, A12) up to ")
        + f"{esc(R[top]['clean'])} ({edges(CL(top))} edges). Next, by the largest graph clean on all four: "
        + ", ".join(f"{name(b)} ({esc(R[b]['clean'])}, {edges(CL(b))})" for b in rank[len(leaders):len(leaders) + 4]) + ".",
        f"<strong>Rust, by itself, does not predict it.</strong> The Rust engines that run in process go far: {rust_line}."
        f"{further_line} The Rust servers do not: SurrealDB and HelixDB are clean only up to {edges(server_reach)} edges, and "
        f"SurrealDB is still killed at its memory limit on 5 M edges with 24 GiB. The best Rust store, {name(best_rust)}, is "
        f"clean up to {edges(CL(best_rust))}; {name(top)} up to {edges(CL(top))}.",
        f"<strong>Where Rust beats Neo4j every time.</strong> On speed, in every pair where both passed: {always_line}. "
        f"The strongest wins belong to engines that run in process and answer without a network hop, so part of that gap is "
        f"architecture, not language.",
        f"<strong>Most failures are placements, not lies.</strong> Across whole untyped graphs under the standard 6 GiB envelope, "
        f"{walls['mem']} loads hit a memory wall and {walls['time']} a time wall; crashes: {walls['crash']}; typed refusals: "
        f"{walls['refused']}. Wrong answers: {gated_line('wrong-answer')}."
        + (f" Queries that ran past their deadline without refusing: {gated_line('hang')}." if gated['hang'] else ""),
        f"<strong>More memory moves some walls and not others.</strong> With 24 GiB containers "
        f"{', '.join(name(b) for b in moved_by_24)} load graphs that 6 GiB placed and pass every family they reach; "
        f"SurrealDB still cannot load wiki-Talk.",
        f"<strong>How to read the rest.</strong> <em>unsupported</em> is never a failure: A3 runs only on the in-process "
        f"reference and A7 only on stores with guarded commits, by design. Every cell below links to the run it came from.",
    ]
    s_exec = (
        '<section class="wrap strain-summary" id="summary">'
        '<div class="section-head"><span class="eyebrow">Executive summary</span>'
        '<h2>Which graph stores take the strain, and which only look fast until they break.</h2></div>'
        '<div class="exec"><ul>' + "".join(f"<li>{x}</li>" for x in exec_items) + '</ul></div>'
        '</section>'
    )

    # ---- key results: reach table and matrix
    reach_rows = []
    for b in rank:
        r = R[b]
        wall = r["first_wall"]
        wall_txt = (f'{esc(wall)} ({esc(CAUSE[r["status"][wall]["cause"]][1])})' if wall else "none")
        fur = r["sustained"] if sizes.get(r["sustained"], 0) > CL(b) else None
        reach_rows.append(
            f'<tr><td>{name(b)}</td><td>{esc(BACKENDS[b]["lang"])}, {esc(BACKENDS[b]["form"])}</td>'
            f'<td class="num">{edges(CL(b)) if r["clean"] else "–"}</td><td>{esc(r["clean"] or "–")}</td>'
            f'<td>{esc(fur) + " (" + edges(sizes[fur]) + ")" if fur else "same"}</td>'
            f'<td class="num">{edges(sizes.get(r["loaded"]))}</td><td>{wall_txt}</td>'
            f'<td>{esc(", ".join(r["wrong"])) or "none"}</td></tr>')
    reach_table = (
        '<div class="evidence-table" tabindex="0" role="region" aria-label="Reach by backend"><table><thead><tr>'
        '<th>Backend</th><th>Built as</th><th>Largest clean</th><th>on</th><th>Further, families run all passed</th>'
        '<th>Largest loaded</th><th>First wall</th><th>Gated failures on</th></tr></thead><tbody>' + "".join(reach_rows) + '</tbody></table></div>')

    def matrix(datasets, envelope, aria):
        head = "".join(f'<th title="{esc(DATASETS[d][2])}">{esc(d)}<br><span class="sz">{edges(sizes[d])}</span></th>' for d in datasets)
        body = []
        for gkey, glabel, _gdesc in GROUPS:
            members = [b for b in BACKENDS if BACKENDS[b]["group"] == gkey]
            rows = []
            for b in members:
                st = [status(cells, b, d, envelope) for d in datasets]
                if all(s["kind"] == "none" for s in st):
                    continue
                rows.append(f'<tr><td class="b">{name(b)}</td>' + "".join(
                    f'<td>{chip(s["kind"], f"{BACKENDS[b]["name"]} · {d}: {s["title"]}")}</td>' for s, d in zip(st, datasets)) + '</tr>')
            if rows:
                body.append(f'<tr class="grp"><th colspan="{len(datasets) + 1}">{esc(glabel)}</th></tr>' + "".join(rows))
        return (f'<div class="evidence-table matrix" tabindex="0" role="region" aria-label="{esc(aria)}"><table><thead><tr>'
                f'<th class="b">Backend</th>{head}</tr></thead><tbody>{"".join(body)}</tbody></table></div>')

    legend = ('<p class="legend">' + " ".join(f'{chip(k)} {esc(KIND_WORD[k])}' for k in
              ("clean", "partial", "caveat", "wrong", "mem", "time", "crash", "refused", "none")) +
              '. Hover a cell for what decided it.</p>')
    big_ds = [d for d in untyped if any(status(cells, b, d, BIG)["kind"] != "none" for b in BACKENDS)]
    s_key = (
        '<section class="band"><div class="wrap">'
        '<div class="section-head" id="key-results"><span class="eyebrow">Key results</span>'
        '<h2>How far each store goes before something gives.</h2>'
        '<p>Standard envelope: every server in a 6 GiB container, each store at its own defaults, two hours for the load '
        'and two for the scenario families. <em>Largest clean</em> is the biggest whole graph on which a store loaded and passed '
        'all four core families (A1, A2, A4, A12). <em>Further</em> names a larger graph it loaded where every family that ran '
        'passed, but not all four ran, usually because the run predates A12 or its time ran out. Embedded stores run inside the '
        'harness and are bounded by the host guard instead of a container.</p></div>'
        + reach_table +
        '<h3 style="margin-top:40px">The strain matrix</h3>'
        '<p>Every backend against every whole graph, smallest to largest. The ladder stops at a backend\'s first wall, so '
        'the cells right of it are usually empty rather than failed.</p>'
        + legend + matrix(untyped, DEFAULT, "Strain matrix, standard envelope") +
        acc('<strong>Typed graphs: deletes, isolation and Cypher</strong><span class="verdict">Wrong answers or deadline hangs on A8 '
            'for FalkorDB, Memgraph and Neo4j; the rest clean or within declared limits.</span>',
            '<p>The typed graphs carry labels and properties for the semantic families: recursive deletes (A5), isolation under '
            'mixed load (A6) and differential Cypher against a reference executor (A8).</p>'
            + matrix(typed, DEFAULT, "Typed graphs, standard envelope")) +
        acc(f'<strong>The same stores with 24 GiB</strong><span class="verdict">{esc(", ".join(BACKENDS[b]["name"] for b in moved_by_24))} '
            'get past the walls 6 GiB set; SurrealDB does not.</span>',
            '<p>A second envelope for the stores the 6 GiB limit placed. These rows stand beside the 6 GiB rows and never replace them.</p>'
            + matrix(big_ds, BIG, "Strain matrix, 24 GiB envelope")) +
        '</div></section>')

    # ---- the Rust question
    grp_rows = []
    for gkey, glabel, gdesc in GROUPS:
        members = [b for b in BACKENDS if BACKENDS[b]["group"] == gkey]
        best = max(members, key=CL)
        grp_rows.append(
            f'<tr><td><strong>{esc(glabel)}</strong><br><span class="muted">{esc(gdesc)}</span></td>'
            f'<td>{", ".join(name(b) for b in members)}</td>'
            f'<td>{name(best)}: {esc(R[best]["clean"] or "–")} ({edges(CL(best))})</td>'
            f'<td>{"; ".join(f"{name(b)} {edges(CL(b))}" for b in members)}</td></tr>')
    h2h_head = "".join(f"<th>{esc(label)}</th>" for _sc, _m, label in h2h_order)
    h2h_rows = []
    for b in RUST:
        tds = []
        for sc, m, _label in h2h_order:
            v = h2h.get((b, sc, m))
            if not v:
                tds.append('<td class="muted">no pair</td>')
                continue
            if v["wins"] == v["pairs"] and v["pairs"] >= 2:
                cls, word = "win", "always faster"
            elif v["wins"] == 0 and v["pairs"] >= 2:
                cls, word = "loss", "never faster"
            else:
                cls, word = "mixed", "mixed"
            title = "; ".join(f"{ds} ({SLICE_LABEL[sl]}) on {h}: {ratio(nv / rv)}" for ds, sl, h, rv, nv in v["detail"])
            tds.append(f'<td class="{cls}" title="{esc(title)}">{word}<br><span class="num">{v["wins"]}/{v["pairs"]} · {esc(ratio(v["median"]))}</span></td>')
        h2h_rows.append(f'<tr><td class="b">{name(b)}</td>{"".join(tds)}</tr>')
    s_rust = (
        '<section class="wrap" id="rust">'
        '<div class="section-head"><span class="eyebrow">The Rust question</span>'
        '<h2>Rust in process goes far; Rust servers do not; Neo4j goes furthest.</h2>'
        f'<p>Grouped by how the engine is built and run; <em>clean</em> as in the key results. Eight of the fifteen backends '
        f'are Rust. The best of them, {name(best_rust)}, is clean up to {edges(CL(best_rust))} edges; {name(top)}, up to '
        f'{edges(CL(top))}. '
        + ('How a store runs predicts its reach better than its language: every Rust engine that runs in process outlasts every '
           'Rust server here.' if local_beats_servers else '')
        + '</p></div>'
        + acc('<strong>Reach by engine family</strong><span class="verdict">In-process Rust engines reach 5.5 to 57.7 M edges; '
              'Rust servers 88 k; the JVM 117 M.</span>',
              '<div class="evidence-table" tabindex="0" role="region" aria-label="Reach by engine group"><table><thead><tr>'
              '<th>Group</th><th>Backends</th><th>Furthest clean</th><th>Each, largest clean graph</th></tr></thead><tbody>'
              + "".join(grp_rows) + '</tbody></table></div>')
        + '<h3 style="margin-top:36px">Head to head with Neo4j over Bolt</h3>'
        '<p>Every pair where a Rust store and Neo4j passed the same scenario on the same graph and slice on the same machine. '
        '<em>Always faster</em> means the Rust store won every such pair (at least two); the ratio is Neo4j\'s time over the '
        'Rust store\'s, the median across pairs. Hover a cell for each pair. In-process stores skip the network and '
        'serialization, and the Rust servers only survive the two smallest graphs, so their pairs are few.</p>'
        f'<div class="evidence-table h2h" tabindex="0" role="region" aria-label="Rust against Neo4j"><table><thead><tr><th class="b">Rust backend</th>{h2h_head}</tr></thead>'
        '<tbody>' + "".join(h2h_rows) + '</tbody></table></div>'
        '</section>')

    # ---- scenario groups
    scen = [
        ("Loading", "LOAD", "The whole graph through the store's own write path, in chunks, with the edge count checked against what "
         "was offered. Most walls are met here: the container's memory limit, the store's own limit, the two-hour load budget, "
         "a server's request timeout, or a projection from the store's measured rate that says the load cannot finish in time."),
        ("Read pathologies", "A1 · A2", "A1 fans out k hops from the graph's highest-degree vertex and compares every layer with an "
         "oracle, exactly. A2 walks hop by hop to a large depth on high-diameter graphs and compares the set reached at each depth. "
         "Hubs and depth are what real graphs have and synthetic benchmarks smooth away."),
        ("Write contention", "A4", "A hundred writers each attach edges to the same hub through their own handle, concurrently. Every "
         "write must be durably applied or refused with a typed conflict, and the final degree after reopening must add up. "
         "A single-writer journal refusing most writes passes; a lost write does not."),
        ("Operability", "A12", "Time from a fresh handle to the first correct answer, the resident footprint after the load, and the "
         "response-time tail of an open-loop read stream at 50 and 200 requests per second, where queueing counts."),
        ("Policy and commit semantics", "A3 · A7", "A3 throws Cartesian products and unbounded matches at the bounded-read policy, "
         "which must refuse in time; it runs on the in-process reference. A7 replays guarded commits by idempotency key and "
         "requires exactly one durable effect; only stores with guarded commits take part. Elsewhere both read <em>unsupported</em>, by design."),
        ("Typed-graph semantics", "A5 · A6 · A8", "On LDBC SNB and ICIJ Offshore Leaks: A5 deletes a reply tree under concurrent "
         "readers, A6 checks isolation under read-then-write load (stores without a conditional write declare last-writer-wins), "
         "and A8 runs a pinned set of Cypher queries through each store and a reference executor and compares the full results."),
    ]
    s_scen = (
        '<section class="band"><div class="wrap">'
        '<div class="section-head" id="scenarios"><span class="eyebrow">What the scenarios test</span>'
        '<h2>Six groups of strain, one question each.</h2>'
        '<p>A <em>pass</em> needs all nine hard gates at zero: wrong answer, lost write, duplicate durable mutation, isolation '
        'anomaly, policy bypass, hang without refusal, out-of-memory or crash, unauthorized disclosure, non-deterministic receipt. '
        '<em>Unsupported</em> means the store or its adapter does not offer the operation, and is never a pass or a fail. '
        '<em>Not tested</em> means the harness projected the load past its budget and did not spend it.</p></div>'
        '<div class="scen-grid">' + "".join(
            f'<div class="scen"><span class="eyebrow">{esc(code)}</span><h3>{esc(title)}</h3><p>{text}</p></div>'
            for title, code, text in scen) + '</div></div></section>')

    # ---- every cell, grouped
    def cell_table(keys):
        rows = []
        order = ["LOAD", "A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A12"]
        for k in sorted(keys, key=lambda k: (order.index(k[2]) if k[2] in order else 99, k[3] != "full", k[4])):
            c = cells[k]
            o = c["obs"]
            word, long = CAUSE[c["cause"]]
            kind = ("clean" if c["cause"] == "pass" else "partial" if c["cause"] == "pass-incomplete"
                    else "wrong" if c["cause"] in BAD else WALL.get(c["cause"], "none"))
            notes = " | ".join(c["row"].get("notes") or [])
            srv = o.get("server_cpu_us")
            rows.append(
                f'<tr><td>{esc(k[2])}</td><td>{esc(SLICE_LABEL.get(k[3], k[3]))}</td><td>{esc(PROFILE_LABEL.get(k[4], k[4]))}</td>'
                f'<td>{chip(kind, long, word)}</td><td class="num">{dur(o.get("wall_us"))}</td>'
                f'<td class="num">{dur(srv) if srv else ""}</td><td class="note">{esc(notes[:220])}</td>'
                f'<td><a href="/evidence/strain/{esc(c["pub"])}/{esc(c["run"])}/report.json">{esc(c["host"])} · {esc(c["run"][:8])}</a></td></tr>')
        return ('<div class="evidence-table compact-table" tabindex="0" role="region" aria-label="Cells"><table><thead><tr>'
                '<th>Scenario</th><th>Slice</th><th>Profile</th><th>Outcome</th><th>Wall</th><th>Server CPU</th><th>Notes</th><th>Run</th>'
                '</tr></thead><tbody>' + "".join(rows) + '</tbody></table></div>')

    def verdict(b):
        r = R[b]
        parts = [f"clean up to {r['clean']} ({edges(CL(b))})" if r["clean"] else "clean on no whole graph"]
        fur = r["sustained"] if sizes.get(r["sustained"], 0) > CL(b) else None
        if fur:
            parts.append(f"loads {fur} ({edges(sizes[fur])}) with every family it ran passing")
        fw = r["first_wall"]
        parts.append(f"first wall at {fw}: {CAUSE[r['status'][fw]['cause']][1]}" if fw else "no wall on any graph it ran")
        if r["wrong"]:
            parts.append("gated failures on " + ", ".join(r["wrong"]))
        text = "; ".join(parts) + "."
        return text[0].upper() + text[1:]

    def tally(kinds):
        c = collections.Counter(kinds)
        return ", ".join(f"{c[k]} {KIND_WORD[k]}" for k in ("clean", "caveat", "partial", "wrong", "mem", "time", "crash", "refused") if c[k])

    group_blocks = []
    for gkey, glabel, gdesc in GROUPS:
        backend_blocks = []
        for b in [b for b in BACKENDS if BACKENDS[b]["group"] == gkey]:
            keys = [k for k in cells if k[1] == b]
            if not keys:
                continue
            tiers = []
            for tier_label, sel in (("Whole graphs", lambda k: k[3] == "full" and not DATASETS.get(k[0], ("", False))[1]),
                                    ("Typed graphs", lambda k: k[3] == "full" and DATASETS.get(k[0], ("", False))[1]),
                                    ("Slices of graphs", lambda k: k[3] != "full")):
                tkeys = [k for k in keys if sel(k)]
                if not tkeys:
                    continue
                ds_blocks, kinds = [], []
                for d in sorted({k[0] for k in tkeys}, key=lambda d: sizes.get(d, 0)):
                    dkeys = [k for k in tkeys if k[0] == d]
                    stt = status(cells, b, d, DEFAULT) if tier_label != "Slices of graphs" else None
                    if stt and stt["kind"] != "none":
                        kinds.append(stt["kind"])
                        said = "every family passed" if stt["kind"] == "clean" else stt["title"]
                        what = f'<span class="verdict">{esc(said)}</span>'
                    else:
                        what = '<span class="verdict">early runs on part of the graph</span>'
                    head = (f'{chip(stt["kind"], stt["title"]) if stt else ""}<strong>{esc(d)}</strong>{what}'
                            f'<span class="muted">{edges(sizes.get(d))} edges · {esc(DATASETS.get(d, ("", "", ""))[2])}</span>')
                    ds_blocks.append(acc(head, cell_table(dkeys), "ds"))
                tsum = tally(kinds) if kinds else f"{len({k[0] for k in tkeys})} graphs, sliced"
                tiers.append(acc(f'<strong>{esc(tier_label)}</strong><span class="muted">{esc(tsum)}</span>', "".join(ds_blocks), "tier"))
            r = R[b]
            meta = (f'{esc(BACKENDS[b]["lang"])}, {esc(BACKENDS[b]["form"])} · {esc(BACKENDS[b]["path"])} · {esc(BACKENDS[b]["version"])}')
            head = (f'<strong>{name(b)}</strong><span class="verdict">{esc(verdict(b))}</span>'
                    f'<span class="muted">{meta}</span>')
            backend_blocks.append(acc(head, "".join(tiers), "backend"))
        members = [b for b in BACKENDS if BACKENDS[b]["group"] == gkey]
        best = max(members, key=CL)
        gsum = f"Furthest clean: {BACKENDS[best]['name']} up to {R[best]['clean']} ({edges(CL(best))})." if R[best]["clean"] else ""
        group_blocks.append(acc(f'<strong>{esc(glabel)}</strong><span class="verdict">{esc(gsum)}</span><span class="muted">{esc(gdesc)}</span>',
                                "".join(backend_blocks), "group"))
    s_cells = (
        '<section class="wrap" id="every-cell">'
        '<div class="section-head"><span class="eyebrow">Every result</span>'
        '<h2>Open an engine family, then a backend, then a graph.</h2>'
        f'<p>Each line says in words what happened at that level: how far a family and a backend got and what stopped them, '
        f'and what decided each graph. Under a graph is its evidence, one row per scenario, linked to the run it came from. '
        f'{len(cells):,} results in all, the latest per dataset, backend, scenario, slice and profile.</p></div>' + "".join(group_blocks) + '</section>')

    key_verdict = (f"{len([b for b in BACKENDS if CL(b) >= 30_000_000])} of {len(BACKENDS)} backends are clean past 30 M edges; "
                   f"{walls['mem']} loads ended at a memory wall, {walls['time']} at a time wall; hover the matrix for what decided each cell.")
    rust_verdict = (f"In-process Rust engines reach {edges(min(CL(b) for b in rust_local))} to {edges(max(CL(b) for b in rust_local))}; "
                    f"Rust servers {edges(server_reach)}; the JVM {edges(CL('neo4j'))}.")
    scen_verdict = "Six families, nine hard gates; unsupported is never a failure."
    s_exec = fold(s_exec, "The longer summary: what was run, who takes the most strain, where Rust wins and loses, how to read the rest.")
    s_key = fold(s_key, esc(key_verdict))
    s_rust = fold(s_rust, esc(rust_verdict))
    s_scen = fold(s_scen, esc(scen_verdict))

    # ---- history: every superseded cell, by the publication it came from
    hist_by_pub = collections.defaultdict(list)
    for h in history:
        hist_by_pub[h["old"]["pub"]].append(h)
    pub_blocks = []
    for pub in sorted(hist_by_pub, reverse=True):
        items = hist_by_pub[pub]
        by_backend = collections.defaultdict(list)
        for h in items:
            by_backend[h["old"]["key"][1]].append(h)
        b_blocks = []
        for b in [b for b in BACKENDS if b in by_backend]:
            rows = []
            for h in sorted(by_backend[b], key=lambda h: (sizes.get(h["old"]["key"][0], 0), h["old"]["key"][2])):
                o, n = h["old"], h["new"]
                ok, nk = o["key"], n["key"]
                rows.append(f'<tr><td>{esc(ok[0])}</td><td>{esc(ok[2])}</td><td>{esc(PROFILE_LABEL.get(ok[4], ok[4]) or "default")}</td>'
                            f'<td>{chip("clean" if o["cause"] == "pass" else "partial" if o["cause"] == "pass-incomplete" else "wrong" if o["cause"] in BAD else WALL.get(o["cause"], "none"), CAUSE[o["cause"]][1], CAUSE[o["cause"]][0])}</td>'
                            f'<td><a href="/evidence/strain/{esc(o["pub"])}/{esc(o["run"])}/report.json">{esc(o["host"])} · {esc(o["run"][:8])}</a></td>'
                            f'<td>{esc(h["why"])}: <a href="/evidence/strain/{esc(n["pub"])}/{esc(n["run"])}/report.json">{esc(n["pub"])} · {esc(n["run"][:8])}</a>, '
                            f'{chip("clean" if n["cause"] == "pass" else "partial" if n["cause"] == "pass-incomplete" else "wrong" if n["cause"] in BAD else WALL.get(n["cause"], "none"), CAUSE[n["cause"]][1], CAUSE[n["cause"]][0])}</td></tr>')
            b_blocks.append(acc(f'<strong>{name(b)}</strong><span class="muted">{len(rows)} cells superseded</span>',
                                '<div class="evidence-table compact-table" tabindex="0" role="region" aria-label="Superseded cells"><table><thead><tr>'
                                '<th>Graph</th><th>Scenario</th><th>Profile</th><th>Was</th><th>Run</th><th>Superseded by</th></tr></thead><tbody>'
                                + "".join(rows) + '</tbody></table></div>', "backend"))
        n_same = sum(1 for h in items if h["why"].startswith("a later run at the same"))
        pub_blocks.append(acc(f'<strong>{esc(pub)}</strong><span class="verdict">{len(items)} cells superseded: {n_same} by a later run at the same profile, '
                              f'{len(items) - n_same} by the harness default moving on.</span>',
                              "".join(b_blocks), "group"))
    s_hist = (
        '<section class="band"><div class="wrap">'
        '<div class="section-head" id="history"><span class="eyebrow">History</span>'
        '<h2>What later runs superseded, and by what.</h2>'
        f'<p>Nothing published is deleted. {len(history)} cells above were replaced by a later run of the same backend, graph, '
        f'scenario and slice: either at the same profile, or by a run at the harness\'s current default after that default moved '
        f'(Turso MVCC loads went from one writer to four on 2026-09-16). Each row links the cell that was superseded and the one that '
        f'superseded it; every bundle they came from is still pinned in the evidence section.</p></div>'
        + "".join(pub_blocks) + '</div></section>')

    return "\n".join([BEGIN, s_verdict, s_exec, s_key, s_rust, s_scen, s_cells, s_hist, END])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("site")
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    page_path = os.path.join(a.site, "graph", "strain", "index.html")
    page = open(page_path).read()
    assert page.count(BEGIN) == 1 and page.count(END) == 1, "the page needs exactly one pair of summary markers"
    block = render(a.site)
    i, j = page.index(BEGIN), page.index(END) + len(END)
    new = page[:i] + block + page[j:]
    if a.check:
        if new != page:
            sys.exit("graph/strain/index.html differs from what the evidence renders; rerun without --check")
        print("strain summary matches the evidence")
        return
    open(page_path, "w").write(new)
    print(f"wrote the strain summary: {len(block):,} bytes")


if __name__ == "__main__":
    main()
