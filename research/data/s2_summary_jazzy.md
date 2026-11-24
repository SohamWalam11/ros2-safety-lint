# Study 2: rosfix on every jazzy repository in rosdistro

826/830 repositories analysed (4 failed). 278 have at least one finding, 65 at least one error-severity finding. Total findings: 3384.

Wall time per repository: median 0.12 s, p95 0.88 s, max 8.31 s (total 217 s, single thread).

| Rule | Severity | Findings | Repositories |
|---|---|---:|---:|
| P003 | warning | 996 | 36 |
| L002 | warning | 761 | 228 |
| X001 | warning | 491 | 2 |
| P002 | error | 215 | 6 |
| L001 | error | 141 | 18 |
| P005 | warning | 121 | 7 |
| C005 | error | 75 | 5 |
| E002 | warning | 71 | 31 |
| S002 | warning | 55 | 16 |
| L004 | warning | 51 | 10 |
| A002 | info | 49 | 18 |
| X002 | warning | 35 | 2 |
| P004 | error | 32 | 7 |
| T004 | info | 30 | 4 |
| L003 | error | 28 | 15 |
| R001 | info | 23 | 12 |
| L005 | error | 23 | 4 |
| T003 | error | 21 | 10 |
| T001 | warning | 20 | 6 |
| A001 | warning | 17 | 3 |
| Q004 | info | 16 | 13 |
| C002 | error | 16 | 3 |
| E001 | error | 15 | 4 |
| C004 | warning | 12 | 4 |
| P001 | warning | 11 | 2 |
| T002 | warning | 9 | 6 |
| S001 | warning | 9 | 3 |
| Q005 | warning | 8 | 1 |
| F002 | error | 8 | 1 |
| N003 | warning | 7 | 2 |
| N005 | warning | 5 | 4 |
| X003 | error | 5 | 2 |
| C001 | error | 4 | 1 |
| C008 | warning | 1 | 1 |
| N006 | warning | 1 | 1 |
| N001 | error | 1 | 1 |
| M001 | warning | 1 | 1 |

Triage sample: 216 findings (≤8 per rule, ≤2 per repository per rule) -> data/s2_triage/sample.jsonl