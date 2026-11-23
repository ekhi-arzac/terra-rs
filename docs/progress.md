# Progress

| Milestone | Status | Evidence |
| --- | --- | --- |
| 0 — Reference analysis | Complete for the initial deliverable | Identified the exact managed binary and release, documented disassembly and subsystem entry points, built the workspace. |
| 1 — Protocol foundation | Complete for the documented join packet set | Bounded frames, versioned typed codecs, guarded session transitions, fixture/round-trip/malformed tests for all 14 packet types, property tests and fragmented/coalesced input tests. |
| 2 — Connectivity | Implemented through character upload. Official client verification pending. | Configured Tokio listener, password negotiation, slot selection and reuse, headless direct-IP tests, six live comparisons against the supplied server. |
| 3–7 | Incomplete | World transmission, gameplay, administration and performance work remain. |

The Rust request-world handler explicitly disconnects at the first unsupported world step. A matching graphical client is required to exercise the connection UI and verify the join milestone.

## Next engineering tasks

1. Exercise the current build with a stock 1.4.5.8 client and preserve a direct-IP join trace.
2. Map each world field consumed by packet 7 to validated disk metadata or server-owned runtime state. Capture packet 7 from the same world on the oracle.
3. Implement the section request and transfer sequence with a bounded tile decoder, send tracking and differential tests through spawn.
4. Establish one simulation owner before admitting players to the active world. Add movement, interactions, persistence and benchmarks as those paths become executable.
