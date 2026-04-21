# Known Issues

## Fan-out performance degrades O(N·M) with large consumer registries

`AffinityRouter` scores every registered consumer for every message routed. With N consumers and M messages, `registry.plan()` runs N scoring operations per message, followed by an O(N log N) sort and an O(N) observe pass over all skipped consumers. At 1 000 consumers × 100 000 messages this is ~100 M scoring operations, yielding ~4 k msg/s drain throughput (debug build).

**Root causes:**
- Full registry scan + sort on every `drain()` call regardless of how many consumers are actually eligible.
- `registry.observe()` iterates all N−1 skipped entries even when they have no dims (empty inner loop but outer iteration still runs).
- No winner cache: a stable topology re-plans identically for every message.

**Potential fixes (independent, stackable):**
- Replace `sort` with a linear-max scan; use partial sort only when a fallback chain is needed.
- Skip the observe pass for consumers with empty dim vecs.
- Add a sticky winner cache keyed on a registry generation counter — O(1) hot path for stable single-winner topologies.
