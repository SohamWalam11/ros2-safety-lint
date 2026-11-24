# Study 1: recall on real configuration fixes

Population: 4489 candidate commits; Set A (flagged) 172, Set B (random) 300.
Inter-rater agreement on `misconfig_fix` (n=94): kappa = 0.892, raw = 0.9468.

| Metric | Value |
|---|---|
| Precision of flagged commits (finding in touched package disappeared) | 93/147 = 63.3% |
| Recall, all genuine fixes | 4.2% [3.7%, 4.8%] (82 / ~1951) |
| Recall, statically detectable fixes | 4.4% [3.9%, 5.1%] (82 / ~1850) |
| Recall, excluding CMakeLists-only commits | 7.3% [6.2%, 9.1%] (80 / ~1095) |
| Recall, silent failures only | 4.2% [2.9%, 7.0%] (9 / ~214) |
| Recall without knowledge packs, all | 4.7% [4.2%, 5.4%] (92 / ~1951) |
| Recall without knowledge packs, static | 5.0% [4.4%, 5.7%] (92 / ~1850) |

## Genuine fixes by ISSTA category (weighted estimate for the population) and detections

| Category | Est. genuine fixes | Detected | Recall [95% CI] |
|---|---:|---:|---|
| Packaging | 1600 | 57 | 3.6% [3.1%, 4.2%] |
| Parameters | 139 | 5 | 3.6% [2.2%, 7.5%] |
| Launch | 69 | 9 | 13.1% [6.4%, 35.4%] |
| Names | 46 | 2 | 4.3% [1.9%, 66.7%] |
| Embedded DSL | 46 | 2 | 4.3% [1.9%, 66.7%] |
| Other Challenges | 29 | 0 | 0.0% [0.0%, 0.0%] |
| Nodes | 20 | 6 | 29.4% [12.2%, 100.0%] |
| Timeliness | 2 | 1 | 50.0% [50.0%, 50.0%] |

## How the pre-fix mistake shows up (weighted)

| Failure | Est. count |
|---|---:|
| build | 1336 |
| loud | 384 |
| silent | 214 |
| unknown | 16 |

## Flagged commits per rule

| Rule | Flagged | Genuine fix |
|---|---:|---:|
| A001 | 3 | 0 |
| A002 | 6 | 6 |
| C004 | 1 | 1 |
| C008 | 1 | 1 |
| F002 | 1 | 1 |
| L001 | 14 | 13 |
| L002 | 132 | 84 |
| L003 | 2 | 0 |
| L004 | 5 | 4 |
| L005 | 2 | 1 |
| N003 | 3 | 3 |
| N006 | 1 | 1 |
| P002 | 2 | 2 |
| P003 | 33 | 19 |
| P005 | 34 | 0 |
| R001 | 8 | 6 |
| S001 | 9 | 7 |
| S002 | 14 | 3 |
| T001 | 17 | 8 |
| T002 | 9 | 0 |