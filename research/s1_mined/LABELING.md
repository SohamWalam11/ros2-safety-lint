# Study 1 labelling guide (blind)

You see one commit at a time: repository, subject/body, touched files, and diff. You do **not**
see any tool output. Decide what the commit fixes, using only the commit itself.

## Fields

| Field | Values | Meaning |
|---|---|---|
| `misconfig_fix` | `yes` / `no` / `unsure` | `yes` only if the parent commit contained a **configuration that was wrong** (would make the robot software fail, misbehave, not build/install, or silently ignore something), and this commit corrects it. `no` for new features, new options, parameter *tuning* of working values, refactors, cosmetics, comments, licence/version/CI changes, adding tests. |
| `category` | one of the 12 below, or `none` | ISSTA 2024 high-level misconfiguration category (Canelas et al.). `none` when `misconfig_fix` is `no`. |
| `subcategory` | from the list below, or free text | Most specific matching sub-category. |
| `failure` | `silent` / `loud` / `build` / `unknown` | How the *pre-fix* mistake shows up: `silent` = runs without error but wrong/ignored behaviour; `loud` = runtime error, crash, exception, refusal to start; `build` = colcon build/install fails or the file is not installed; `unknown`. |
| `static` | `yes` / `no` | Could a tool that reads only the workspace files (launch, YAML, URDF, package.xml, CMake, setup.py, node source) in principle detect the mistake, without running the robot or knowing the physical world? Calibration values, tuning, hardware facts → `no`. |
| `rationale` | ≤ 25 words | What was wrong. |

## Categories and sub-categories (ISSTA 2024, Fig. 3)

- **Messages**: Format; No Publisher; Conflicting Publishers; Filters; Periodic; Frequency; Stale Data; Synchronization (QoS mismatches go here, sub-category `QoS`)
- **Launch**: Includes; Duplication; Race Conditions; Arguments; Environment variables
- **Parameters**: Missing; Default; Dead Writes; Incorrect; Dependency (a key that is never read — typo, wrong node key, wrong nesting, removed parameter — is `Dead Writes`; wrong type/value is `Incorrect`)
- **Semantic Types**: Constraints; Color Format; Pointcloud Transformations; Image Transformations; Physical Units
- **Names**: Mismatches; Namespaces; Remapping
- **Nodes**: Missing; Incorrect; Nodelet manager (wrong executable/plugin name goes here)
- **Timeliness**: (timing assumptions between components; use_sim_time inconsistencies go here, sub-category `Simulation time`)
- **TF**: Incorrect Transform; Missing Transform; Duplication of transforms
- **Embedded DSL**: Navigation Stack; URDF; MoveIt!; ROS Control; Robot Localization
- **Calibration**: Cameras; Odometry; PID controller parameters
- **Contextual**: Simulation vs. Real; Application-Environment; Actuators; Sensors; Mechanical; Compute
- **Other Challenges**: Documentation; BUG:Component; BUG:Infrastructure; Simulation

Packaging mistakes (missing `<depend>`, file not installed, wrong `setup.py`/`setup.cfg` entry
point) are not in the ISSTA taxonomy: use `category: "Packaging"` with sub-category
`Dependency`, `Install`, or `Entry point`.

## Rules of thumb

- Decide from the diff. If the message says "fix" but the diff only adds an option, it is `no`.
- Removing a dependency that is unused is `no` (cleanup). Adding a missing one is `yes`.
- Renaming a parameter because upstream renamed it (migration) is `yes`, `Parameters/Dead Writes`.
- When the diff is truncated and you cannot tell, use `unsure`.
