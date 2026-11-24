# rosfix mutation benchmark

## PX4-Autopilot

Clean analysis: 6 findings, 1336 ms (25 files parsed).

Findings on the clean workspace (to be labelled manually): O001=6

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## autoware

Clean analysis: 0 findings, 12 ms (0 files parsed).

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## moveit2

Clean analysis: 58 findings, 1322 ms (487 files parsed).

Findings on the clean workspace (to be labelled manually): L002=32, O001=5, O002=20, U002=1

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| drop-dependency | L002 | 5 | 5 | 100% |
| exec-name-typo | L001 | 4 | 4 | 100% |
| ros-parameters-typo | P002 | 3 | 3 | 100% |

## ros-planning__navigation2

Clean analysis: 63 findings, 1541 ms (778 files parsed).

Findings on the clean workspace (to be labelled manually): A002=2, L002=49, N009=1, P005=1, S002=9, T003=1

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| double-as-int | P004 | 4 | 4 | 100% |
| drop-dependency | L002 | 3 | 3 | 100% |
| exec-name-typo | L001 | 11 | 11 | 100% |
| nav2-plugin-typo | N002 | 20 | 20 | 100% |
| param-key-abbrev | P003 | 3 | 3 | 100% |
| param-key-typo | P003 | 20 | 20 | 100% |
| ros-parameters-typo | P002 | 16 | 16 | 100% |

## open-rmf__rmf_ros2

Clean analysis: 5 findings, 363 ms (135 files parsed).

Findings on the clean workspace (to be labelled manually): L002=2, P003=3

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## ros2_control

Clean analysis: 0 findings, 595 ms (124 files parsed).

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| ros-parameters-typo | P002 | 11 | 11 | 100% |

## ros2__rmw_fastrtps

Clean analysis: 0 findings, 68 ms (16 files parsed).

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## ros2__demos

Clean analysis: 10 findings, 141 ms (145 files parsed).

Findings on the clean workspace (to be labelled manually): E002=2, L002=8

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| drop-dependency | L002 | 4 | 4 | 100% |
| exec-name-typo | L001 | 12 | 12 | 100% |
| ros-parameters-typo | P002 | 3 | 3 | 100% |

## ros2__system_tests

Clean analysis: 2 findings, 124 ms (75 files parsed).

Findings on the clean workspace (to be labelled manually): L008=2

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## turtlebot3

Clean analysis: 10 findings, 61 ms (63 files parsed).

Findings on the clean workspace (to be labelled manually): E002=2, L002=8

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| drop-dependency | L002 | 4 | 4 | 100% |
| exec-name-typo | L001 | 1 | 1 | 100% |
| nav2-plugin-typo | N002 | 20 | 20 | 100% |
| ros-parameters-typo | P002 | 17 | 17 | 100% |

## ros2__sros2

Clean analysis: 0 findings, 17 ms (20 files parsed).

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## ros2__rmw_cyclonedds

Clean analysis: 0 findings, 59 ms (1 files parsed).

| mutation | expected rule | injected | detected | recall |
|---|---|---:|---:|---:|
| (no mutation sites found) | | 0 | 0 | |

## Total

| mutation | injected | detected | recall |
|---|---:|---:|---:|
| double-as-int | 4 | 4 | 100% |
| drop-dependency | 16 | 16 | 100% |
| exec-name-typo | 28 | 28 | 100% |
| nav2-plugin-typo | 40 | 40 | 100% |
| param-key-abbrev | 3 | 3 | 100% |
| param-key-typo | 20 | 20 | 100% |
| ros-parameters-typo | 50 | 50 | 100% |
| **all** | **161** | **161** | **100%** |

## Fix assistant

For mutations with a known correct value: how often the right value was among the options, and what the lexical ranker and the built-in learned ranker chose (correct / abstained / wrong). Abstaining is safe: the finding is still reported.

| mutation | cases | answer among options | lexical | learned |
|---|---:|---:|---|---|
| exec-name-typo | 28 | 28 | 12 / 16 / 0 | 28 / 0 / 0 |
| param-key-abbrev | 3 | 3 | 3 / 0 / 0 | 3 / 0 / 0 |
| param-key-typo | 20 | 19 | 0 / 20 / 0 | 0 / 20 / 0 |

Findings on clean workspaces by rule: A002=2, E002=4, L002=99, L008=2, N009=1, O001=11, O002=20, P003=3, P005=1, S002=9, T003=1, U002=1

