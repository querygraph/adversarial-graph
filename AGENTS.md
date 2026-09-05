# Agent guidance for adversarial-graph

## Benchmark Neutrality

- This benchmark is neutral at every level: dataset choice, scenario design,
  harness code, evidence bundles, documentation, commit messages, and the
  strain ledger on adversari.al. It measures and reports; it does not pursue
  an outcome for any system, including the Grust stack it was written beside.
- Do not write, commit, or publish strategy about making one system look
  better or worse than another, anywhere. State goals as engineering
  properties (exact answers, hard gates, disclosed transports and read
  paths, recorded resource envelopes), never as a contest against a named
  engine.
- Every failure stays visible: a later pass never erases an earlier failing
  cell, superseded runs are named in the superseding row, and a system that
  does not implement an operation is `unsupported`, not a pass and not a
  fail.
- If a document or message is found to violate this, fix it and remove the
  violation from history rather than leaving it with a disclaimer.

## Working rules

- The harness depends on published `grust-graph` crates and on Grust internal
  adapters pinned to a release tag by a `git` dependency; never on a local
  Grust checkout.
- Results are generated (`scripts/render-results.py`), never hand-edited;
  site evidence is frozen by `scripts/bundle-site-evidence.py` and verified
  independently on the site.
- Wall times on a shared host are upper bounds; compare the CPU columns and
  the recorded load average, and say so wherever a number is quoted.
