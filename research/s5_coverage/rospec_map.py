"""Map ROSpec's expected errors (examples/expected/*.txt) to rosfix coverage."""
import collections, json, re, sys
from pathlib import Path
exp = Path(sys.argv[1])
CLASSES = [  # (regex on the first error, class, verdict, rosfix rule / gap)
    (r"Publisher not found for subscriber", "topic without publisher", "covered", "T005 (T001 if a near-miss name exists)"),
    (r"Provider not found for consumes service", "service/action without server", "covered", "T005"),
    (r"count\(publishers\(", "topic with more than one publisher where one is required", "gap", "new: single-writer topics"),
    (r"QoSError", "QoS mismatch", "covered", "Q001"),
    (r"has multiple parents", "TF frame with two parents", "partial", "F001 sees URDF and static_transform_publisher; runtime broadcasters (amcl, robot_localization, SLAM) need a TF-broadcaster catalog"),
    (r"is cyclic", "TF cycle", "covered", "F005"),
    (r"Broadcaster not found for listener", "TF lookup of a frame nobody broadcasts", "partial", "F004 for frames named in parameters; needs the TF-broadcaster catalog"),
    (r"is_simulation -> use_sim_time", "simulation without use_sim_time", "covered", "S002 (when the simulator is launched)"),
    (r"ParameterDependencyError", "cross-parameter dependency", "gap", "roadmap #7: cross-parameter constraints"),
    (r"Configurable information .* was not found", "parameter the node does not declare", "covered", "P003"),
    (r"is missing from the component instance", "required parameter missing", "partial", "P005 only when declared without default"),
    (r"Refinement", "deployment/context assumption", "out of scope", "depends on the environment, not the files"),
    (r"already defined", "two nodes with the same name", "covered", "A003"),
]
rows = []
for f in sorted(exp.glob("expected-*.txt")):
    qid = f.stem.split("-", 1)[1]
    text = f.read_text(encoding="utf-8", errors="replace")
    for rx, cls, verdict, rule in CLASSES:
        if re.search(rx, text):
            rows.append(dict(id=qid, cls=cls, verdict=verdict, rule=rule, rospec=text.strip().splitlines()[0][:160]))
            break
    else:
        rows.append(dict(id=qid, cls="?", verdict="?", rule="", rospec=text[:160]))
json.dump(rows, open("rospec_map.json", "w"), indent=1)
print(len(rows), collections.Counter(r["verdict"] for r in rows))
for c, n in collections.Counter((r["cls"], r["verdict"]) for r in rows).most_common(): print(n, c)
