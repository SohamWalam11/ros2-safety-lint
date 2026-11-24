# Coverage against published misconfiguration benchmarks

Two questions reviewers ask of any ROS misconfiguration checker: which kinds of mistakes from
the published taxonomy does it cover, and how does it compare with the closest specification-
based approach. This page answers both for rosfix as of November 2026 (93 rules, 18 agents).

Reproduce: `python -I research/s5_coverage/rospec_map.py <rospec>/examples/expected` with a
clone of https://github.com/pcanelas/rospec (Apache-2.0; commit e47d6eb used here). Study 1
counts come from `research/data/s1_labels/labels_batch_*.json` and `det_ids.json`.

## 1. The ISSTA 2024 taxonomy

Canelas et al., "Understanding Misconfigurations in ROS" (ISSTA 2024) group misconfigurations
reported on ROS Answers into 12 categories. The table maps each sub-category to the rosfix
rules that address it and gives the evidence from Study 1 (472 labelled fix commits from 190
ROS 2 repositories): genuine fixes in the sample of that sub-category / of those statically
detectable / detected by rosfix. Packaging is not part of the ISSTA taxonomy; it is added
because it dominates real fixes.

Status: covered (a rule targets it), partial (a rule exists but sees only some cases, usually
for lack of knowledge about nodes outside the workspace), gap (statically detectable in
principle, no rule yet), out of scope (needs the running system, the hardware or the intent).

| Category / sub-category | rosfix rules | Status | Study 1 |
|---|---|---|---|
| Messages / Format | T003 | partial | |
| Messages / No Publisher | T005, T001 | covered | |
| Messages / Conflicting Publishers | none | gap | |
| Messages / Filters | none | gap | |
| Messages / Periodic, Frequency | C010 (controller rates only) | partial | |
| Messages / Stale Data | none | out of scope | |
| Messages / Synchronization (QoS) | Q001-Q005 | covered | |
| Launch / Includes | L005, A001 | covered | 3 / 3 / 3 |
| Launch / Duplication | A003 | covered | |
| Launch / Race Conditions | M001, M002 (lifecycle only) | partial | |
| Launch / Arguments | A001, A002 | covered | 8 / 8 / 5 |
| Launch / Environment variables | D001-D004, `rosfix doctor` | covered | |
| Launch / syntax, malformed XML, merge markers | R001 | covered | 3 / 3 / 3 |
| Parameters / Missing | P005 | partial | 5 / 5 / 3 |
| Parameters / Default | none | out of scope | |
| Parameters / Dead Writes | P001, P002, P003, P007, Q005, N006 | covered | 11 / 11 / 4 |
| Parameters / Incorrect | P004, P006, C009 | partial | 2 / 2 / 0 |
| Parameters / Dependency | none | gap | |
| Semantic Types / Constraints | P006, C009 | partial | |
| Semantic Types / Color Format | none | gap | |
| Semantic Types / Pointcloud, Image Transformations | none | out of scope | |
| Semantic Types / Physical Units | none | gap | |
| Names / Mismatches | T001 | covered | 3 / 3 / 2 |
| Names / Namespaces | T004, P001 | covered | 1 / 1 / 1 |
| Names / Remapping | T002 | covered | 2 / 2 / 0 |
| Nodes / Missing | L001, M002, C007 | covered | |
| Nodes / Incorrect | L001, N002, N008, C003 | covered | 7 / 7 / 6 |
| Nodes / Nodelet manager (ROS 2: components) | L001, L006, L010 | covered | |
| Timeliness / Simulation time | S001-S003 | covered | 2 / 2 / 2 |
| TF / Incorrect Transform | F002, F003 | partial | |
| TF / Missing Transform | F004 | partial | |
| TF / Duplication | F001, F005 (cycles) | partial | |
| Embedded DSL / Navigation Stack | N001-N010 (incl. BehaviorTree XML) | covered | |
| Embedded DSL / URDF | C005, F001, N003, U001-U004 | partial | 3 / 2 / 1 |
| Embedded DSL / MoveIt! | V001-V005 | covered | 1 / 1 / 1 |
| Embedded DSL / ROS Control | C001-C010 | covered | 2 / 2 / 1 |
| Embedded DSL / Robot Localization | P003 through the knowledge pack | partial | |
| Calibration (all) | none | out of scope | |
| Contextual (all) | none | out of scope | |
| Other Challenges (all) | none | out of scope | 2 / 2 / 0 |
| Packaging / Dependency | L002, L009 | covered | 143 / 137 / 56 |
| Packaging / Install | L004, L010, L011, L012 | partial | 30 / 30 / 4 |
| Packaging / Entry point | L003, L001 | covered | |
| Packaging / build sources, config, flags, CMake API | none | gap | 8 / 8 / 1 |

Study 1 counts are from the labelled sample, not population estimates (the sample over-
represents commits rosfix flagged; see `research/data/s1_results.md` for weighted recall).
Empty cells: no genuine fix of that sub-category in the sample.

What the table says. Of the 12 categories, rosfix has rules for 9; Calibration, Contextual and
Other Challenges are out of reach of any file-reading tool. Within the covered categories the
largest measured gap is packaging: 30 install fixes in the sample, 4 detected, and 8 build-
configuration fixes, 1 detected. Detection within "covered" sub-categories is incomplete
mainly because nodes from packages outside the workspace and the knowledge packs are treated
as unknown, and rosfix stays silent about them by design.

## 2. ROSpec's evaluation questions

ROSpec (Canelas et al., OOPSLA 2025) is a specification language: component authors write
what a node requires, and the ROSpec checker verifies a system against those specifications.
Its evaluation modelled questions from ROS Answers; the published artifact has specifications
for 84 of them, 61 labelled "detectable" (the specification catches the problem) and 23
"documentation" (the answer was a documentation problem, not a checkable rule).

The questions come from ROS 1 systems and the artifact contains specifications, not the
original launch, YAML and source files, so rosfix cannot be run on them directly. Instead
each detectable question is classified by the error ROSpec reports for it
(`examples/expected/expected-<id>.txt`) and mapped to the rosfix rule that targets the same
mistake in a ROS 2 workspace. Each question gets the class of its first error, in the order of
the table below. This is an upper bound for "covered": the rule exists, but on a real
workspace it fires only when rosfix can see both sides of the problem.

| ROSpec error | Questions | rosfix | Status |
|---|---:|---|---|
| topic without publisher | 13 | T005 (T001 when a near-miss name exists) | covered |
| service or action without server | 4 | T005 | covered |
| simulation without `use_sim_time` | 3 | S002, when the simulator is launched | covered |
| parameter the node does not declare | 2 | P003 | covered |
| QoS mismatch | 1 | Q001 | covered |
| TF frame with two parents | 8 | F001 sees URDF joints and static_transform_publisher; amcl, robot_localization and SLAM broadcast at runtime | partial |
| TF lookup of a frame nobody broadcasts | 3 | F004 for frames named in parameters | partial |
| required parameter missing | 1 | P005 when declared without default | partial |
| cross-parameter dependency | 16 | none | gap |
| topic with more than one publisher where one is required | 7 | none | gap |
| TF cycle | 1 | F005 | covered |
| two nodes with the same name | 1 | A003 | covered |
| deployment or environment assumption | 2 | none | out of scope |
| total | 62 | | |

(62 rather than 61: the artifact has expected outputs for two ids that describe the same
question, 53228 and 53288.)

Summary: 25 of 62 (40%) are mistakes rosfix has a rule for (two of them, F005 and A003, added
after this comparison found them missing), 12 (19%) are partially covered, 23 (37%) are
statically detectable mistakes without a rosfix rule, and 2 (3%) depend on the deployment. The comparison is not symmetric. ROSpec needs a hand-written specification for
every component involved, written here by the tool's authors after reading the answer; rosfix
needs none and infers what it checks from code, launch files and knowledge packs. The 23 gaps
are exactly the knowledge a specification adds: which parameters constrain each other, which
topics must have a single writer, and which nodes broadcast which transforms.

## 3. What this adds to the roadmap

| Gap | Questions | Plan |
|---|---:|---|
| cross-parameter dependencies (robot_localization `*_config` vs `*_differential`, `two_d_mode`, voxel layer `z_voxels`, AMCL alpha limits, footprint vs obstacle height) | 16 | roadmap item 7: a constraint table in the knowledge packs |
| runtime TF broadcasters (amcl: map to odom; robot_localization: world_frame to base_link_frame; SLAM toolbox; robot_state_publisher from the URDF) | 11 | a TF-broadcaster catalog keyed on parameters, used by F001 and F004 |
| single-writer topics (`joint_states` from both a driver and joint_state_publisher, two `cmd_vel` sources, two point-cloud drivers) | 7 | a catalog of topics that must have one publisher, plus an option in rosfix.toml |

Sources: Canelas et al., ISSTA 2024, doi:10.1145/3650212.3680350; Canelas et al., OOPSLA 2025,
doi:10.1145/3763169; ROSpec evaluation page https://rospec.pcanelas.com/evaluation.

## Appendix: per-question mapping

| ROS Answers id | ROSpec error class | Status | rosfix |
|---|---|---|---|
| 108291 | required parameter missing | partial | P005 only when declared without default |
| 11095 | TF lookup of a frame nobody broadcasts | partial | F004 for frames named in parameters; needs the TF-broadcaster catalog |
| 11547 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 12977 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 142456 | QoS mismatch | covered | Q001 |
| 164526 | service/action without server | covered | T005 |
| 165661 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 185909 | deployment/context assumption | out of scope | depends on the environment, not the files |
| 188919 | simulation without use_sim_time | covered | S002 (when the simulator is launched) |
| 190126 | service/action without server | covered | T005 |
| 190483 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 195289 | service/action without server | covered | T005 |
| 200387 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 204321 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 210583 | deployment/context assumption | out of scope | depends on the environment, not the files |
| 214816 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 217551 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 218573 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 22195 | parameter the node does not declare | covered | P003 |
| 223521 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 225375 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 225694 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 227092 | TF lookup of a frame nobody broadcasts | partial | F004 for frames named in parameters; needs the TF-broadcaster catalog |
| 227128 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 229722 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 231458 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 232853 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 235375 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 250138 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 253726 | two nodes with the same name | covered | A003 |
| 255713 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 257637 | TF lookup of a frame nobody broadcasts | partial | F004 for frames named in parameters; needs the TF-broadcaster catalog |
| 257935 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 259561 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 280412 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 28163 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 283932 | TF cycle | covered | F005 |
| 285062 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 296720 | simulation without use_sim_time | covered | S002 (when the simulator is launched) |
| 316037 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 319065 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 320389 | topic with more than one publisher where one is required | gap | new: single-writer topics |
| 325712 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 33175 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 336221 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 340683 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 350740 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 360477 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 364801 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 378462 | parameter the node does not declare | covered | P003 |
| 379101 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 390380 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 393902 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 401185 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 405765 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 41000 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
| 45556 | service/action without server | covered | T005 |
| 53228 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 53288 | topic without publisher | covered | T005 (T001 if a near-miss name exists) |
| 63179 | cross-parameter dependency | gap | roadmap #7: cross-parameter constraints |
| 66176 | simulation without use_sim_time | covered | S002 (when the simulator is launched) |
| 98736 | TF frame with two parents | partial | F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog |
