//! BehaviorTree.CPP XML files (Nav2 behavior trees): which node types a tree uses.

use crate::facts::BtFile;
use crate::source::{FileId, Loc};

/// A BehaviorTree.CPP XML file: `<root …>` with `<BehaviorTree>` children.
pub fn is_relevant(text: &str) -> bool {
    text.contains("<BehaviorTree") && text.contains("<root")
}

/// Elements that are structure, not node types.
const STRUCTURE: &[&str] = &[
    "root",
    "BehaviorTree",
    "TreeNodesModel",
    "include",
    "input_port",
    "output_port",
    "inout_port",
    "Metadata",
    "Item",
];

/// Generic element names whose node type is the `ID` attribute.
const GENERIC: &[&str] = &[
    "Action",
    "Condition",
    "Control",
    "Decorator",
    "SubTree",
    "SubTreePlus",
];

pub fn extract(file: FileId, text: &str) -> Result<BtFile, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let mut f = BtFile {
        file,
        ..Default::default()
    };
    for n in doc.descendants().filter(|n| n.is_element()) {
        let tag = n.tag_name().name();
        let loc = Loc::new(
            file,
            n.range().start,
            n.range().end.min(n.range().start + tag.len() + 1),
        );
        if n.ancestors().any(|a| a.has_tag_name("TreeNodesModel")) {
            // Custom node types declared for Groot: `<Action ID="X"/>` or `<X/>`.
            if let Some(id) = n.attribute("ID") {
                f.declared.push(id.to_string());
            } else if !STRUCTURE.contains(&tag) && !GENERIC.contains(&tag) {
                f.declared.push(tag.to_string());
            }
            continue;
        }
        match tag {
            "BehaviorTree" => {
                if let Some(id) = n.attribute("ID") {
                    f.trees.push(id.to_string());
                }
            }
            "include" => f.has_include = true,
            "SubTree" | "SubTreePlus" => {
                if let Some(id) = n.attribute("ID") {
                    f.subtree_refs.push((id.to_string(), loc));
                }
            }
            t if GENERIC.contains(&t) => {
                if let Some(id) = n.attribute("ID") {
                    f.nodes.push((id.to_string(), loc));
                }
            }
            t if STRUCTURE.contains(&t) => {}
            t => {
                // Inside a <BehaviorTree>: the element name is the node type.
                if n.ancestors().any(|a| a.has_tag_name("BehaviorTree")) {
                    f.nodes.push((t.to_string(), loc));
                }
            }
        }
    }
    Ok(f)
}

/// Node types BehaviorTree.CPP itself provides (v3 and v4).
pub const BUILTIN_NODES: &[&str] = &[
    "Sequence",
    "SequenceStar",
    "SequenceWithMemory",
    "ReactiveSequence",
    "Fallback",
    "ReactiveFallback",
    "Parallel",
    "ParallelAll",
    "IfThenElse",
    "WhileDoElse",
    "Switch2",
    "Switch3",
    "Switch4",
    "Switch5",
    "Switch6",
    "ManualSelector",
    "Inverter",
    "ForceSuccess",
    "ForceFailure",
    "Repeat",
    "RetryUntilSuccessful",
    "RetryUntilSuccesful",
    "KeepRunningUntilFailure",
    "Delay",
    "Timeout",
    "RunOnce",
    "Precondition",
    "AlwaysSuccess",
    "AlwaysFailure",
    "AlwaysRunning",
    "SetBlackboard",
    "UnsetBlackboard",
    "Script",
    "ScriptCondition",
    "Sleep",
    "LoopInt",
    "LoopDouble",
    "LoopString",
    "LoopBool",
    "WasEntryUpdated",
    "SkipUnlessUpdated",
    "WaitValueUpdate",
    "EntryUpdated",
    "BlackboardCheckInt",
    "BlackboardCheckDouble",
    "BlackboardCheckString",
    "BlackboardCheckBool",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_types_subtrees_and_models() {
        let xml = r#"<root BTCPP_format="4" main_tree_to_execute="Main">
  <BehaviorTree ID="Main">
    <PipelineSequence name="nav">
      <RateController hz="1.0"><ComputePathToPose goal="{goal}" path="{path}"/></RateController>
      <Action ID="FollowPath" path="{path}"/>
      <SubTree ID="Recovery"/>
    </PipelineSequence>
  </BehaviorTree>
  <TreeNodesModel><Action ID="MyCustom"/><Condition ID="IsDocked"/></TreeNodesModel>
</root>"#;
        let f = extract(0, xml).unwrap();
        let ids: Vec<&str> = f.nodes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "PipelineSequence",
                "RateController",
                "ComputePathToPose",
                "FollowPath"
            ]
        );
        assert_eq!(f.trees, vec!["Main"]);
        assert_eq!(f.subtree_refs[0].0, "Recovery");
        assert_eq!(f.declared, vec!["MyCustom", "IsDocked"]);
    }
}
